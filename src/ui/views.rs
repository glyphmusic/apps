//! Main-area views: home, search, library, queue panel. Content floats
//! directly on the background; only individual cards are flat surfaces.

use crate::app::{Glyph, ListRef, SearchFilter};
use crate::ui::lists::{artist_link, list_cover, with_list_menu, with_menu};
use crate::i18n::{t, tf};
use crate::theme;
use crate::ui::widgets::*;
use glyph_protocol::Track;
use gpui::{div, prelude::*, px, Context, FontWeight, Window};
use std::rc::Rc;

fn scroll_area(id: &'static str) -> gpui::Stateful<gpui::Div> {
    div().id(id).size_full().overflow_y_scroll().px(px(32.0)).py(px(20.0))
}

fn track_list(app: &mut Glyph, tracks: Vec<Track>, prefix: &'static str, cx: &mut Context<Glyph>) -> gpui::Div {
    let list = Rc::new(tracks);
    let rows: Vec<_> = (0..list.len()).map(|i| track_row(app, list.clone(), i, prefix, cx)).collect();
    div().flex().flex_col().gap(px(2.0)).children(rows)
}

fn greeting() -> gpui::SharedString {
    // Local hour is not available without a tz crate; keep it neutral.
    t("home.greeting")
}

// ------------------------------------------------------------------ home

pub fn home(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let recent: Vec<Track> = app.library.recent.iter().take(12).cloned().collect();
    let cards = theme::on_panel(|| home_cards(app, cx));
    let row = if recent.is_empty() {
        ascii_state(ART_EMPTY, t("home.empty.title"), t("home.empty.sub")).into_any_element()
    } else {
        cover_row(app, recent, "home-row", cx).into_any_element()
    };

    let recs: Vec<Track> = app.recs.iter().filter(|t| app.allowed(t)).cloned().collect();
    let trending: Vec<Track> = app.trending.iter().filter(|t| app.allowed(t)).cloned().collect();
    let recs_row = (!recs.is_empty()).then(|| cover_row_with(app, recs, "home-recs", false, cx));
    let trend_row = (!trending.is_empty()).then(|| cover_row_with(app, trending, "home-trend", false, cx));

    scroll_area("home-scroll").track_scroll(&app.main_scroll).child(
        div()
            .flex()
            .flex_col()
            .gap(px(24.0))
            .child(heading(greeting()))
            .child(cards)
            .child(section_title("disc-3", t("home.recent")))
            .child(row)
            .when_some(recs_row, |d, r| d.child(section_title("heart", t("home.recs"))).child(r))
            .when_some(trend_row, |d, r| d.child(section_title("headphones", t("home.trending"))).child(r)),
    )
}

/// Flat dark cards, 56 px cover left, title + subtitle.
fn home_cards(app: &mut Glyph, cx: &mut Context<Glyph>) -> gpui::Div {
    let n = app.library.liked.len();
    let base = |id: gpui::ElementId| {
        card()
            .id(id)
            .flex()
            .items_center()
            .gap(px(14.0))
            .h(px(56.0))
            .w(px(320.0))
            .overflow_hidden()
            .cursor_pointer()
            .hover(|s| s.bg(theme::card_hover()))
    };
    let mut cards = vec![with_list_menu(base("card-liked".into()), ListRef::Liked, cx)
        .on_click(cx.listener(|this, _, _, cx| this.open_list(ListRef::Liked, cx)))
        .child(
            div()
                .size(px(56.0))
                .flex()
                .items_center()
                .justify_center()
                // GPUI clips to rectangles, so round the tile's own corners.
                .rounded_l(px(8.0))
                .bg(theme::active())
                .child(icon("heart", 20.0, theme::text())),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(div().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).child(t("library.liked")))
                .child(div().text_size(theme::text_sm()).text_color(theme::text_dim()).child(tf("common.n_songs", &[("n", &n)]))),
        )
        .into_any_element()];
    // Playlists open; songs play.
    let pls: Vec<(String, String, usize)> =
        app.library.playlists.iter().take(5).map(|p| (p.id.clone(), p.name.clone(), p.tracks.len())).collect();
    for (i, (id, name, count)) in pls.into_iter().enumerate() {
        let l = ListRef::User(id);
        let art = list_cover(app, &l, 56.0, cx);
        cards.push(
            with_list_menu(base(("card-pl", i).into()), l.clone(), cx)
                .on_click(cx.listener(move |this, _, _, cx| this.open_list(l.clone(), cx)))
                .child(art)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .gap(px(2.0))
                        .child(div().truncate().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).child(name))
                        .child(div().truncate().text_size(theme::text_sm()).text_color(theme::text_dim()).child(tf("common.n_songs", &[("n", &count)]))),
                )
                .into_any_element(),
        );
    }
    let room = 6usize.saturating_sub(cards.len());
    let list = Rc::new(app.library.recent.iter().take(room).cloned().collect::<Vec<_>>());
    for i in 0..list.len() {
        let tr = list[i].clone();
        let l = list.clone();
        let art = cover(app, &tr, 56.0, 8.0, cx);
        let card = base(("card", i).into()).on_click(cx.listener(move |this, _, _, cx| this.play_list((*l).clone(), i, cx)));
        let artist = artist_link(("card-artist", i), &tr.artist, cx);
        cards.push(
            with_menu(card, tr.clone(), Some((ListRef::Recent, i)), cx)
                .child(art)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .gap(px(2.0))
                        .child(song_title(tr.title.clone(), tr.explicit).text_size(theme::text_md()).font_weight(FontWeight::MEDIUM))
                        .child(div().flex().min_w_0().text_size(theme::text_sm()).text_color(theme::text_dim()).child(artist)),
                )
                .into_any_element(),
        );
    }
    div().flex().flex_wrap().gap(px(10.0)).children(cards)
}

/// Horizontal row of square covers (176 px) with title / artist below.
fn cover_row(app: &mut Glyph, tracks: Vec<Track>, prefix: &'static str, cx: &mut Context<Glyph>) -> gpui::Stateful<gpui::Div> {
    cover_row_with(app, tracks, prefix, true, cx)
}

/// Row of big covers; `recent`: the menu offers "Remove from recently played".
fn cover_row_with(app: &mut Glyph, tracks: Vec<Track>, prefix: &'static str, recent: bool, cx: &mut Context<Glyph>) -> gpui::Stateful<gpui::Div> {
    let list = Rc::new(tracks);
    let items: Vec<_> = (0..list.len())
        .map(|i| {
            let tr = list[i].clone();
            let l = list.clone();
            let art = cover(app, &tr, 176.0, 8.0, cx);
            let artist = artist_link((prefix, 10_000 + i), &tr.artist, cx);
            let el = div()
                .id((prefix, i))
                .flex_none()
                .w(px(192.0))
                .p(px(8.0))
                .rounded(px(10.0))
                .flex()
                .flex_col()
                .gap(px(4.0))
                .cursor_pointer()
                .hover(|s| s.bg(gpui::hsla(0.0, 0.0, 1.0, 0.05)))
                .on_click(cx.listener(move |this, _, _, cx| this.play_list((*l).clone(), i, cx)))
                .child(art)
                .child(song_title(tr.title.clone(), tr.explicit).pt(px(4.0)).text_size(theme::text_md()).font_weight(FontWeight::MEDIUM))
                .child(div().flex().min_w_0().text_size(theme::text_sm()).text_color(theme::text_dim()).child(artist));
            with_menu(el, tr, recent.then_some((ListRef::Recent, i)), cx)
        })
        .collect();
    let h = handle_for(prefix);
    div().id(prefix).track_scroll(&h).flex().gap(px(4.0)).ml(px(-8.0)).overflow_x_scroll().child(smooth_wheel(&h, true, cx)).children(items)
}

// ------------------------------------------------------------------ search

pub fn search(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let filter = app.search.filter.unwrap_or(SearchFilter::All);
    let chip_btn = |id: &'static str, key: &str, f: SearchFilter, cx: &mut Context<Glyph>| {
        div()
            .id(id)
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.search.filter = Some(f);
                cx.notify();
            }))
            .child(chip(t(key), filter == f))
    };
    let chips = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(chip_btn("f-all", "search.filter.all", SearchFilter::All, cx))
        .child(chip_btn("f-tracks", "search.filter.tracks", SearchFilter::Tracks, cx))
        .child(chip_btn("f-albums", "search.filter.albums", SearchFilter::Albums, cx))
        .child(chip_btn("f-artists", "search.filter.artists", SearchFilter::Artists, cx));

    let query = app.search.query.clone();
    let body: gpui::AnyElement = if let Some(e) = app.search.error.clone() {
        ascii_state(ART_OFFLINE, e, t("search.error.sub")).into_any_element()
    } else if query.trim().is_empty() {
        ascii_state(ART_SEARCH, t("search.empty.title"), t("search.empty.sub")).into_any_element()
    } else if app.search.results.is_empty() && app.search.loading {
        div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .pt(px(8.0))
            .when(filter == SearchFilter::All, |d| d.child(skeleton_artist_rows("search", 2)))
            .child(skeleton_rows("search", 7))
            .into_any_element()
    } else if app.search.results.is_empty() {
        ascii_state(ART_EMPTY, t("search.none.title"), format!("\u{201c}{query}\u{201d}")).into_any_element()
    } else {
        match filter {
            SearchFilter::All | SearchFilter::Tracks => {
                let r = app.search.results.clone();
                // Matching artists on top (Spotify-style mixed list), then songs.
                let artists = if filter == SearchFilter::All { app.search_artists(3) } else { Vec::new() };
                let artist_rows: Vec<_> = artists
                    .into_iter()
                    .enumerate()
                    .map(|(i, (name, tr))| {
                        let k = reveal_k(&format!("search:artist:{name}"), 0.9);
                        let art = cover(app, &tr, 48.0, 24.0, cx);
                        let shown = decode(&name, k);
                        let follow = crate::ui::lists::follow_button(("search-follow", i), &name, Some(tr.clone()), app, 30.0, cx);
                        let n2 = name.clone();
                        crate::ui::lists::with_artist_menu(div().id(("search-artist", i)), name.clone(), cx)
                            .flex()
                            .items_center()
                            .gap(px(14.0))
                            .px(px(10.0))
                            .py(px(6.0))
                            .rounded(px(8.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::card()))
                            .on_click({
                                let tr2 = tr.clone();
                                cx.listener(move |this, _, w, cx| this.pick_search_artist(&n2, Some(tr2.clone()), w, cx))
                            })
                            .child(art)
                            .child(div().flex_1().min_w_0().truncate().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).text_color(theme::text()).child(shown))
                            .child(kind_chip(t("search.kind.artist")))
                            .child(follow)
                    })
                    .collect();
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .children(artist_rows)
                    .child(track_list(app, r, "search-row", cx))
                    .into_any_element()
            }
            SearchFilter::Albums | SearchFilter::Artists => {
                let mut groups: Vec<(String, String, Vec<Track>)> = Vec::new();
                for tr in &app.search.results {
                    let (k, sub) = if filter == SearchFilter::Albums {
                        match &tr.album {
                            Some(al) => (al.clone(), tr.artist.clone()),
                            None => continue,
                        }
                    } else {
                        (tr.artist.clone(), String::new())
                    };
                    match groups.iter_mut().find(|g| g.0 == k) {
                        Some(g) => g.2.push(tr.clone()),
                        None => groups.push((k, sub, vec![tr.clone()])),
                    }
                }
                let items: Vec<_> = groups
                    .into_iter()
                    .enumerate()
                    .map(|(i, (name, sub, tracks))| {
                        let n = tracks.len();
                        let is_album = filter == SearchFilter::Albums;
                        let (title, sub2) = (name.clone(), sub.clone());
                        let art = cover(app, &tracks[0], 44.0, if filter == SearchFilter::Artists { 22.0 } else { 6.0 }, cx);
                        let gm = if is_album {
                            Some(ListRef::Album { title: name.clone(), artist: sub.clone(), tracks: tracks.clone() })
                        } else {
                            None
                        };
                        let row = div().id(("group", i));
                        let row = match gm {
                            Some(l) => with_list_menu(row, l, cx),
                            None => row,
                        };
                        row
                            .flex()
                            .items_center()
                            .gap(px(14.0))
                            .px(px(10.0))
                            .py(px(6.0))
                            .rounded(px(8.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::card()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if is_album {
                                    let l = ListRef::Album { title: title.clone(), artist: sub2.clone(), tracks: tracks.clone() };
                                    this.open_list(l, cx)
                                } else {
                                    this.open_artist(&title, cx)
                                }
                            }))
                            .child(art)
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .child(div().truncate().text_size(theme::text_md()).child(name))
                                    .child(div().truncate().text_size(theme::text_sm()).text_color(theme::text_dim()).child(sub)),
                            )
                            .child(div().text_size(theme::text_sm()).text_color(theme::text_dim()).child(tf("common.n_songs", &[("n", &n)])))
                    })
                    .collect();
                div().flex().flex_col().gap(px(2.0)).children(items).into_any_element()
            }
        }
    };
    // Query completions as chips: click to search for that.
    let completions = app.search_completions(6);
    let suggest = (!completions.is_empty()).then(|| {
        div().flex().flex_wrap().gap(px(8.0)).children(completions.into_iter().enumerate().map(|(i, c)| {
            let q = c.clone();
            div()
                .id(("search-sugg", i))
                .flex()
                .items_center()
                .gap(px(6.0))
                .h(px(28.0))
                .px(px(10.0))
                .rounded(px(8.0))
                .bg(gpui::hsla(0.0, 0.0, 1.0, 0.06))
                .border_1()
                .border_color(gpui::hsla(0.0, 0.0, 1.0, 0.06))
                .cursor_pointer()
                .text_size(px(12.5))
                .text_color(gpui::hsla(0.0, 0.0, 1.0, 0.85))
                .hover(|s| s.bg(gpui::hsla(0.0, 0.0, 1.0, 0.11)))
                .on_click(cx.listener(move |this, _, _, cx| this.search_for(q.clone(), cx)))
                .child(icon("search", 12.0, gpui::hsla(0.0, 0.0, 1.0, 0.5)))
                .child(c)
        }))
    });
    let s = &app.search;
    let footer = div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .when(s.loading && !s.results.is_empty(), |d| d.child(spinner_line(app.spinner(), t("search.more"))))
        .children(s.errors.iter().map(|(p, m)| status_line(format!("[{}] {} -> {}", p.badge(), p.display_name(), m))));

    scroll_area("search-scroll").track_scroll(&app.main_scroll).child(
        div()
            .flex()
            .flex_col()
            .gap(px(18.0))
            .child(heading(if query.trim().is_empty() { t("nav.search") } else { format!("\u{201c}{query}\u{201d}").into() }))
            .child(chips)
            .children(suggest)
            .child(body)
            .child(footer),
    )
}

// ------------------------------------------------------------------ queue

pub fn queue_panel(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let up = app.upcoming();
    let n = up.len();
    let current = app.current.clone();
    let now = current.map(|tr| {
        let art = cover(app, &tr, 40.0, 6.0, cx);
        div()
            .px(px(14.0))
            .pb(px(12.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(art)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(div().truncate().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).child(tr.title))
                    .child(div().truncate().text_size(theme::text_sm()).text_color(theme::text_dim()).child(tr.artist)),
            )
    });
    panel()
        .flex_none()
        .w(px(320.0))
        .h_full()
        .bg(theme::panel_strong())
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .px(px(14.0))
                .py(px(10.0))
                .child(div().flex_1().text_size(theme::text_lg()).font_weight(FontWeight::SEMIBOLD).child(t("queue.title")))
                .child(icon_button("q-close", "x", 16.0, theme::text_dim()).on_click(cx.listener(|this, _, _, cx| {
                    this.show_queue = false;
                    cx.notify();
                }))),
        )
        .children(now)
        .child(div().px(px(14.0)).pb(px(6.0)).text_size(theme::text_sm()).text_color(theme::text_dim()).child(tf("queue.next_n", &[("n", &n)])))
        .child(fade_edges(div()
                .id("q-scroll")
                .track_scroll(&app.queue_scroll)
                .size_full()
                .overflow_y_scroll()
                .px(px(6.0))
                .when(!app.queue_ok, |d| d.child(ascii_state(ART_OFFLINE, t("status.queue_offline"), "")))
                .when(app.queue_ok && n == 0, |d| {
                    d.child(ascii_state(ART_EMPTY, t("queue.empty"), if app.radio { t("queue.radio_next") } else { "".into() }))
                })
                .children(up.into_iter().enumerate().map(|(i, tr)| {
                    div()
                        .id(("q", i))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(8.0))
                        .py(px(6.0))
                        .rounded(px(6.0))
                        .hover(|s| s.bg(theme::card()))
                        .child(mono().w(px(22.0)).text_size(theme::text_xs()).text_color(theme::text_faint()).child(format!("{:02}", i + 1)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(song_title(tr.title.clone(), tr.explicit).text_size(theme::text_sm()))
                                .child(div().truncate().text_size(theme::text_xs()).text_color(theme::text_dim()).child(tr.artist.clone())),
                        )
                        .child(icon_button(("q-up", i), "arrow-up", 14.0, theme::text_dim()).on_click(cx.listener(move |this, _, _, cx| {
                            if i > 0 {
                                this.queue_move(i, i - 1, cx)
                            }
                        })))
                        .child(icon_button(("q-down", i), "arrow-down", 14.0, theme::text_dim()).on_click(cx.listener(move |this, _, _, cx| {
                            if i + 1 < n {
                                this.queue_move(i, i + 1, cx)
                            }
                        })))
                        .child(icon_button(("q-rm", i), "x", 14.0, theme::text_dim()).on_click(cx.listener(move |this, _, _, cx| this.queue_remove(i, cx))))
                })), theme::panel_strong(), 18.0, Some(&app.queue_scroll)).flex_1().min_h_0().child(scrollbar(&app.queue_scroll, true, cx)))
        .child(
            div()
                .px(px(14.0))
                .py(px(12.0))
                .border_t_1()
                .border_color(theme::border_faint())
                .child(toggle("q-radio", app.radio, t("queue.radio")).on_click(cx.listener(|this, _, _, cx| {
                    this.radio = !this.radio;
                    cx.notify();
                }))),
        )
}
