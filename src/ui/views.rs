//! Main-area views: home, search, library, queue panel. Content floats
//! directly on the background; only individual cards are flat surfaces.

use crate::app::{Glyph, ListRef, SearchFilter};
use crate::ui::lists::{artist_link, list_cover, with_artist_menu, with_list_menu, with_menu};
use crate::i18n::{t, tf};
use crate::theme;
use crate::ui::widgets::*;
use glyph_core::RecentItem;
use glyph_protocol::Track;
use gpui::{div, prelude::*, px, AnyElement, Context, Div, FontWeight, SharedString, Window};
use std::rc::Rc;

fn scroll_area(id: &'static str) -> gpui::Stateful<gpui::Div> {
    div().id(id).size_full().overflow_y_scroll().touch_axis().px(px(theme::PAGE_X)).pt(px(theme::PAGE_TOP)).pb(px(20.0))
}

fn track_list(app: &mut Glyph, tracks: Vec<Track>, prefix: &'static str, cx: &mut Context<Glyph>) -> gpui::Div {
    let list = Rc::new(tracks);
    // Rows are 56 px + 2 px gap; only the visible ones are built.
    let scroll = app.main_scroll.clone();
    virtual_rows(prefix, list.len(), 58.0, &scroll, |i| track_row(app, list.clone(), i, prefix, cx).into_any_element())
}

fn greeting() -> gpui::SharedString {
    // Local hour is not available without a tz crate; keep it neutral.
    t("home.greeting")
}

// ------------------------------------------------------------------ home

pub fn home(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let recent: Vec<RecentItem> = app.library.recent.iter().take(12).cloned().collect();
    let cards = theme::on_panel(|| home_cards(app, cx));
    let has_recent = !recent.is_empty();
    let row = if recent.is_empty() {
        ascii_state(ART_EMPTY, t("home.empty.title"), t(if crate::ui::compact::TOUCH { "home.empty.sub.touch" } else { "home.empty.sub" })).into_any_element()
    } else {
        recent_row(app, recent, "home-row", cx).into_any_element()
    };

    let recs: Vec<Track> = app.recs.iter().filter(|t| app.allowed(t)).cloned().collect();
    let trending: Vec<Track> = app.trending.iter().filter(|t| app.allowed(t)).cloned().collect();
    let (recs_all, trend_all) = (recs.clone(), trending.clone());
    let recs_row = (!recs.is_empty()).then(|| cover_row_with(app, recs, "home-recs", false, cx));
    let trend_row = (!trending.is_empty()).then(|| cover_row_with(app, trending, "home-trend", false, cx));
    let releases = app.new_releases.clone();
    let release_row = (!releases.is_empty()).then(|| new_releases_row(app, releases, cx));

    scroll_area("home-scroll").track_scroll(&app.main_scroll).child(
        div()
            .flex()
            .flex_col()
            .gap(px(if crate::ui::compact::TOUCH { 20.0 } else { 24.0 }))
            // Phones: the greeting sits in the header beside the profile picture.
            .when(!crate::ui::compact::TOUCH, |d| d.child(heading(greeting())))
            .child(cards)
            .child(section(section_head("disc-3", t("home.recent"), has_recent.then_some(More::Recent), cx), row))
            .when_some(release_row, |d, r| d.child(section(section_head("sparkles", t("home.new_releases"), None, cx), r)))
            .when_some(recs_row, |d, r| d.child(section(section_head("heart", t("home.recs"), Some(More::Songs("home-recs-all", recs_all)), cx), r)))
            .when_some(trend_row, |d, r| d.child(section(section_head("headphones", t("home.trending"), Some(More::Songs("home-trend-all", trend_all)), cx), r))),
    )
}

/// What "Show all" of a section opens.
enum More {
    Recent,
    Songs(&'static str, Vec<Track>),
}

/// A section: its title sits close above its content.
fn section(head: impl IntoElement, body: impl IntoElement) -> Div {
    div().flex().flex_col().gap(px(if crate::ui::compact::TOUCH { 6.0 } else { 10.0 })).child(head).child(body)
}

/// Section title with "Show all" on the right.
fn section_head(icon_name: &str, title: SharedString, more: Option<More>, cx: &mut Context<Glyph>) -> Div {
    let link = more.map(|m| {
        let id: &'static str = match &m {
            More::Recent => "home-recent-all",
            More::Songs(id, _) => id,
        };
        let title2 = title.clone();
        text_button(id, t("home.show_all"))
            .text_size(theme::text_sm())
            .font_weight(FontWeight::MEDIUM)
            .on_click(cx.listener(move |this, _, _, cx| match &m {
                More::Recent => this.open_list(ListRef::Recent, cx),
                More::Songs(_, tracks) => this.open_section(title2.to_string(), tracks.clone(), cx),
            }))
    });
    div().flex().items_center().child(section_title(icon_name, title)).child(div().flex_1()).children(link)
}

/// A Home section on its own page ("Show all").
pub fn section_view(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let (title, tracks) = app.section.clone().unwrap_or_default();
    let back = crate::ui::lists::back_button(app, cx);
    // One of these songs is on: the button pauses / resumes it.
    let here = app.current.as_ref().is_some_and(|c| {
        let k = crate::app::track_key(c);
        tracks.iter().any(|t| crate::app::track_key(t) == k)
    });
    let (playing, loading) = (here && app.is_playing(), here && app.is_loading());
    let t_play = tracks.clone();
    let play = (!tracks.is_empty()).then(|| {
        crate::ui::lists::round_play("section-play", playing, loading, 44.0).on_click(cx.listener(move |this, _, _, cx| {
            if here {
                this.toggle_play(cx)
            } else {
                this.play_list(t_play.clone(), 0, cx)
            }
        }))
    });
    scroll_area("section-scroll")
        .track_scroll(&app.main_scroll)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(16.0))
                .children(back)
                // Round play button on the left of the heading, like Spotify.
                .child(div().flex().items_center().gap(px(14.0)).children(play).child(heading(title)))
                .child(track_list(app, tracks, "section-row", cx)),
        )
}

thread_local! {
    /// Width the home cards had last frame (they share it out as a grid).
    static HOME_CARDS_W: std::cell::Cell<f32> = const { std::cell::Cell::new(1000.0) };
}

/// Home cards at least this wide; they grow to fill the row up to the max.
/// Phones get two columns (Spotify) instead of a tall stack of six.
const HOME_CARD_MIN: f32 = if crate::ui::compact::TOUCH { 150.0 } else { 260.0 };
const HOME_CARD_MAX: f32 = 420.0;
const HOME_CARD_GAP: f32 = if crate::ui::compact::TOUCH { 8.0 } else { 10.0 };

/// Flat dark cards, 56 px cover left, title + subtitle. They fill the width
/// as a grid: as many columns as fit at the minimum width, each stretched up
/// to the maximum.
fn home_cards(app: &mut Glyph, cx: &mut Context<Glyph>) -> gpui::Div {
    let n = app.library.liked.len();
    let base = |id: gpui::ElementId, key: String| {
        card()
            .id(id)
            .flex()
            .items_center()
            .gap(px(if crate::ui::compact::TOUCH { 10.0 } else { 14.0 }))
            .h(px(56.0))
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .cursor_pointer()
            .relative()
            .child(hover_fill(key, gpui::hsla(0.0, 0.0, 1.0, 0.04), 8.0))
    };
    let mut cards = vec![with_list_menu(base("card-liked".into(), "card-liked".into()), ListRef::Liked, cx)
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
    let active: Vec<Track> = app.active_home_tracks();
    // Up to two playlists you listened to lately come first, then active
    // songs; other playlists fill whatever is left.
    let hot: Vec<String> = app.active_home_playlists().into_iter().take(2).collect();
    let songs = active.len().min(5 - hot.len());
    let mut ids = hot.clone();
    ids.extend(app.library.playlists.iter().filter(|p| !hot.contains(&p.id)).map(|p| p.id.clone()).take(5 - hot.len() - songs));
    let pls: Vec<(String, String, usize)> =
        ids.iter().filter_map(|id| app.library.playlist(id)).map(|p| (p.id.clone(), p.name.clone(), p.tracks.len())).collect();
    for (i, (id, name, count)) in pls.into_iter().enumerate() {
        let l = ListRef::User(id);
        let art = list_cover(app, &l, 56.0, cx);
        cards.push(
            with_list_menu(base(("card-pl", i).into(), format!("card-pl-{i}")), l.clone(), cx)
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
    let list = Rc::new(active.into_iter().take(room).collect::<Vec<_>>());
    for i in 0..list.len() {
        let tr = list[i].clone();
        let l = list.clone();
        let art = cover(app, &tr, 56.0, 8.0, cx);
        let card = base(("card", i).into(), format!("card-{i}")).on_click(cx.listener(move |this, _, _, cx| this.play_list((*l).clone(), i, cx)));
        let artist = artist_link(("card-artist", i), &tr.artist, cx);
        cards.push(
            with_menu(card, tr.clone(), None, cx)
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
    let w = HOME_CARDS_W.with(|c| c.get());
    let cols = (((w + HOME_CARD_GAP) / (HOME_CARD_MIN + HOME_CARD_GAP)).floor() as usize).clamp(1, cards.len().max(1));
    let max_w = cols as f32 * HOME_CARD_MAX + (cols - 1) as f32 * HOME_CARD_GAP;
    div()
        .w_full()
        .relative()
        // Measures the room; a change lays the grid out again next frame.
        .child(
            gpui::canvas(
                |b, window, _| {
                    let w = f32::from(b.size.width);
                    if (HOME_CARDS_W.with(|c| c.replace(w)) - w).abs() > 0.5 {
                        window.request_animation_frame();
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
        .child(div().grid().grid_cols(cols as u16).gap(px(HOME_CARD_GAP)).w_full().max_w(px(max_w)).children(cards))
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
                .relative()
                .child(hover_fill(format!("{prefix}-{i}"), gpui::hsla(0.0, 0.0, 1.0, 0.05), 10.0))
                .on_click(cx.listener(move |this, _, _, cx| this.play_list((*l).clone(), i, cx)))
                .child(art)
                .child(song_title(tr.title.clone(), tr.explicit).pt(px(4.0)).text_size(theme::text_md()).font_weight(FontWeight::MEDIUM))
                .child(div().flex().min_w_0().text_size(theme::text_sm()).text_color(theme::text_dim()).child(artist));
            with_menu(el, tr, recent.then_some((ListRef::Recent, i)), cx)
        })
        .collect();
    let h = handle_for(prefix);
    div().id(prefix).track_scroll(&h).flex().gap(px(4.0)).ml(px(-8.0))
        // Rows run to the panel edge instead of stopping at the page padding.
        .mr(px(-32.0))
        .pr(px(32.0))
        .overflow_x_scroll().touch_axis().child(smooth_wheel(&h, true, cx)).children(items)
}

/// "New from artists you follow": release cards (cover, title, artist,
/// "SINGLE · 2026-10-12"), opening the release's song list.
fn new_releases_row(app: &mut Glyph, releases: Vec<glyph_core::providers::deezer::Release>, cx: &mut Context<Glyph>) -> gpui::Stateful<gpui::Div> {
    const PREFIX: &str = "home-new";
    let items: Vec<AnyElement> = releases
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            let seed = r.cover_track();
            let art = cover(app, &seed, 176.0, 8.0, cx);
            let artist = artist_link((PREFIX, 10_000 + i), &r.artist, cx);
            let kind = match r.kind {
                glyph_core::catalog::AlbumKind::Album => t("album.kind.album"),
                glyph_core::catalog::AlbumKind::Ep => t("album.kind.ep"),
                glyph_core::catalog::AlbumKind::Single => t("album.kind.single"),
                glyph_core::catalog::AlbumKind::Compilation => t("album.kind.compilation"),
            };
            let label = match r.released.as_deref() {
                Some(day) => format!("{kind} · {day}"),
                None => kind.to_string(),
            };
            let lm = ListRef::Album { title: r.title.clone(), artist: r.artist.clone(), tracks: vec![seed] };
            let title = r.title.clone();
            with_list_menu(div().id((PREFIX, i)), lm, cx)
                .flex_none()
                .w(px(192.0))
                .p(px(8.0))
                .rounded(px(10.0))
                .flex()
                .flex_col()
                .gap(px(4.0))
                .cursor_pointer()
                .relative()
                .child(hover_fill(format!("{PREFIX}-{i}"), gpui::hsla(0.0, 0.0, 1.0, 0.05), 10.0))
                .on_click(cx.listener(move |this, _, _, cx| this.open_release(r.clone(), cx)))
                .child(art)
                .child(div().pt(px(4.0)).truncate().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).text_color(theme::text()).child(title))
                .child(div().flex().min_w_0().text_size(theme::text_sm()).text_color(theme::text_dim()).child(artist))
                .child(mono().text_size(px(11.0)).text_color(theme::text_faint()).child(label.to_uppercase()))
                .into_any_element()
        })
        .collect();
    let h = handle_for(PREFIX);
    div().id(PREFIX).track_scroll(&h).flex().gap(px(4.0)).ml(px(-8.0))
        .mr(px(-32.0))
        .pr(px(32.0))
        .overflow_x_scroll().touch_axis().child(smooth_wheel(&h, true, cx)).children(items)
}

/// "Recently played" row: songs and artists (a round picture, opening the
/// artist page) side by side, newest first.
fn recent_row(app: &mut Glyph, items: Vec<RecentItem>, prefix: &'static str, cx: &mut Context<Glyph>) -> gpui::Stateful<gpui::Div> {
    let list = Rc::new(items);
    let cards: Vec<AnyElement> = (0..list.len())
        .map(|i| match list[i].clone() {
            RecentItem::Song { track } => {
                let tr = track.clone();
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
                    .relative()
                    .child(hover_fill(format!("{prefix}-{i}"), gpui::hsla(0.0, 0.0, 1.0, 0.05), 10.0))
                    .on_click(cx.listener(move |this, _, _, cx| this.play_recent_item(i, cx)))
                    .child(art)
                    .child(song_title(tr.title.clone(), tr.explicit).pt(px(4.0)).text_size(theme::text_md()).font_weight(FontWeight::MEDIUM))
                    .child(div().flex().min_w_0().text_size(theme::text_sm()).text_color(theme::text_dim()).child(artist));
                with_menu(el, tr, Some((ListRef::Recent, 0)), cx).into_any_element()
            }
            RecentItem::Artist { name, track } => {
                let art: AnyElement = match track.as_ref() {
                    Some(tr) => cover(app, tr, 176.0, 88.0, cx).into_any_element(),
                    None => div()
                        .flex_none()
                        .size(px(176.0))
                        .rounded_full()
                        .bg(theme::card())
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon("user-round", 56.0, gpui::hsla(0.0, 0.0, 1.0, 0.55)))
                        .into_any_element(),
                };
                let n = name.clone();
                with_artist_menu(
                    div()
                        .id((prefix, i))
                        .flex_none()
                        .w(px(192.0))
                        .p(px(8.0))
                        .rounded(px(10.0))
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .cursor_pointer()
                        .relative()
                        .child(hover_fill(format!("{prefix}-{i}"), gpui::hsla(0.0, 0.0, 1.0, 0.05), 10.0))
                        .on_click(cx.listener(move |this, _, _, cx| this.open_artist(&n, cx)))
                        .child(art)
                        .child(div().truncate().pt(px(4.0)).text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).child(name.clone()))
                        .child(div().text_size(theme::text_sm()).text_color(theme::text_dim()).child(t("search.kind.artist"))),
                    name.clone(),
                    cx,
                )
                .into_any_element()
            }
        })
        .collect();
    let h = handle_for(prefix);
    div().id(prefix).track_scroll(&h).flex().gap(px(4.0)).ml(px(-8.0))
        // Rows run to the panel edge instead of stopping at the page padding.
        .mr(px(-32.0))
        .pr(px(32.0))
        .overflow_x_scroll().touch_axis().child(smooth_wheel(&h, true, cx)).children(cards)
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
    } else if query.trim().is_empty() && crate::ui::compact::TOUCH {
        crate::ui::compact::search_recent(app, cx)
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
                            .relative()
                            .child(hover_fill(format!("search-artist-{i}"), theme::card(), 8.0))
                            .on_click({
                                let tr2 = tr.clone();
                                cx.listener(move |this, _, w, cx| this.pick_search_artist(&n2, Some(tr2.clone()), w, cx))
                            })
                            .child(art)
                            .when(crate::ui::compact::TOUCH, |d| d.px(px(0.0)).gap(px(12.0)))
                            .when(!crate::ui::compact::TOUCH, |d| {
                                d.child(crate::ui::lists::artist_name_badged(app, &name, shown.clone(), 15.0, cx).flex_1().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).text_color(theme::text()))
                                    .child(kind_chip(t("search.kind.artist")))
                                    .child(follow)
                            })
                            // Phones: name over "Artist", like the song rows.
                            .when(crate::ui::compact::TOUCH, |d| {
                                d.child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .gap(px(2.0))
                                        .child(crate::ui::lists::artist_name_badged(app, &name, shown.clone(), 15.0, cx).text_size(px(15.0)).text_color(theme::text()))
                                        .child(div().text_size(px(12.5)).text_color(theme::text_dim()).child(t("search.kind.artist"))),
                                )
                            })
                    })
                    .collect();
                // Songs found through a line of their lyrics, with that line.
                let hits = app.search.lyric_hits.clone();
                let lyric_section = (!hits.is_empty()).then(|| {
                    let list = Rc::new(hits.iter().map(|(t, _)| t.clone()).collect::<Vec<_>>());
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .pb(px(10.0))
                        .child(div().flex().items_center().gap(px(8.0)).pb(px(4.0)).child(icon("mic-vocal", 15.0, theme::text_dim())).child(div().text_size(theme::text_md()).font_weight(FontWeight::SEMIBOLD).child(t("search.lyrics.title"))))
                        .children(hits.iter().enumerate().map(|(i, (_, line))| {
                            div()
                                .flex()
                                .flex_col()
                                .child(crate::ui::lists::song_row(app, list.clone(), i, "search-lyric-row", None, false, cx))
                                .when(!line.is_empty(), |d| {
                                    d.child(
                                        div()
                                            .pl(px(90.0))
                                            .pb(px(6.0))
                                            .mt(px(-6.0))
                                            .flex()
                                            .items_center()
                                            .gap(px(6.0))
                                            .text_size(theme::text_xs())
                                            .text_color(theme::text_faint())
                                            .child(kind_chip(t("search.lyrics.in")))
                                            .child(div().truncate().italic().child(format!("\u{201e}{line}\u{201c}"))),
                                    )
                                })
                        }))
                });
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .children(artist_rows)
                    .children(lyric_section)
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
                            .relative()
                            .child(hover_fill(format!("search-group-{i}"), theme::card(), 8.0))
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
    // Phones: no completion chips; results start right under the filters.
    let completions = if crate::ui::compact::TOUCH { Vec::new() } else { app.search_completions(6) };
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
                .relative()
                .child(hover_fill(format!("search-sugg-{i}"), gpui::hsla(0.0, 0.0, 1.0, 0.05), 8.0))
                .on_click(cx.listener(move |this, _, _, cx| this.search_for(q.clone(), cx)))
                .child(icon("search", 12.0, gpui::hsla(0.0, 0.0, 1.0, 0.5)))
                .child(c)
        }))
    });
    // Near the end of the list: ask for the next page.
    {
        let h = &app.main_scroll;
        let left = h.max_offset().y + h.offset().y;
        if f32::from(left) < 900.0 && !app.search.results.is_empty() && matches!(filter, SearchFilter::All | SearchFilter::Tracks) {
            let me = cx.entity();
            cx.defer(move |cx| me.update(cx, |g, cx| g.search_load_more(cx)));
        }
    }
    let s = &app.search;
    let footer = div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .when((s.loading || s.loading_more) && !s.results.is_empty(), |d| d.child(spinner_line(app.spinner(), t("search.more"))))
        .children(s.errors.iter().map(|(p, m)| status_line(format!("[{}] {} -> {}", p.badge(), p.display_name(), m))));

    scroll_area("search-scroll").track_scroll(&app.main_scroll).child(
        div()
            .flex()
            .flex_col()
            .gap(px(18.0))
            // Phones: the title sits in the header; filters only with results.
            .when(!crate::ui::compact::TOUCH, |d| d.child(heading(if query.trim().is_empty() { t("nav.search") } else { format!("\u{201c}{query}\u{201d}").into() })))
            .when(!crate::ui::compact::TOUCH || !query.trim().is_empty(), |d| d.child(chips))
            .children(suggest)
            .children(crate::ui::profile::search_hit(app, &query, cx))
            .child(body)
            .child(footer),
    )
}

// ------------------------------------------------------------------ queue

/// Height of one queue row; drop slots are computed from it.
const QUEUE_ROW_H: f32 = 44.0;

thread_local! {
    /// Eased height of the opening above each slot while a row is dragged,
    /// so the rows glide aside instead of jumping.
    /// Per list (queue, profile playlists, ...).
    static QUEUE_GAPS: std::cell::RefCell<std::collections::HashMap<&'static str, (Vec<f32>, Option<std::time::Instant>)>> = Default::default();
}

/// A drag-reorder gap animation (queue or another list) still has distance to go.
pub fn queue_anim_running() -> bool {
    QUEUE_GAPS.with(|g| g.borrow().values().any(|v| v.1.is_some_and(|t| t.elapsed().as_secs_f32() < 0.5)))
}

fn queue_gaps(slots: usize, target: Option<usize>) -> Vec<f32> {
    drag_gaps("queue", QUEUE_ROW_H, slots, target)
}

/// Current gap heights of list `key` for `slots` (+1 for "after the last
/// row"), easing to a full row (`row_h`) at `target`. `None` (no drag)
/// resets at once. Shared by every drag-reorder list.
pub fn drag_gaps(key: &'static str, row_h: f32, slots: usize, target: Option<usize>) -> Vec<f32> {
    QUEUE_GAPS.with(|g| {
        let mut all = g.borrow_mut();
        let g = all.entry(key).or_default();
        let Some(at) = target else {
            *g = (Vec::new(), None);
            return vec![0.0; slots + 1];
        };
        let now = std::time::Instant::now();
        if g.0.len() != slots + 1 {
            // Drag just began: the gap opens where the row was lifted.
            let mut v = vec![0.0; slots + 1];
            v[at.min(slots)] = row_h;
            *g = (v, Some(now));
            return g.0.clone();
        }
        let moving = g.0.iter().enumerate().any(|(i, h)| (h - if i == at { row_h } else { 0.0 }).abs() > 0.5);
        let dt = g.1.map_or(0.016, |t| t.elapsed().as_secs_f32()).clamp(0.0, 0.05);
        let k = 1.0 - (-dt / 0.06).exp();
        for (i, h) in g.0.iter_mut().enumerate() {
            let t = if i == at { row_h } else { 0.0 };
            *h += (t - *h) * k;
        }
        g.1 = if moving { Some(now) } else { g.1.filter(|t| t.elapsed().as_secs_f32() < 0.5) };
        if moving {
            g.1 = Some(now);
        }
        g.0.clone()
    })
}

/// A queue entry being dragged: the row itself follows the cursor.
#[derive(Clone)]
struct QueueDrag {
    from: usize,
    track: Track,
    art: Option<std::path::PathBuf>,
}

/// Grip / number, cover, title + artist: shared by the row and its drag image.
fn queue_row_inner(i: usize, tr: &Track, art: Option<std::path::PathBuf>, group: SharedString, lifted: bool) -> Div {
    let art = match art {
        Some(p) => gpui::img(p).flex_none().size(px(32.0)).rounded(px(4.0)).object_fit(gpui::ObjectFit::Cover).into_any_element(),
        None => div().flex_none().size(px(32.0)).rounded(px(4.0)).bg(theme::card()).into_any_element(),
    };
    let handle = div()
        .flex_none()
        .w(px(18.0))
        .h(px(32.0))
        .relative()
        .flex()
        .items_center()
        .justify_center();
    let hk = hover_k(&group);
    let handle = if lifted {
        handle.child(icon("grip-vertical", 14.0, theme::text()))
    } else {
        handle
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(1.0 - hk)
                    .child(mono().text_size(theme::text_xs()).text_color(theme::text_faint()).child(format!("{:02}", i + 1))),
            )
            .child(div().opacity(hk).child(icon("grip-vertical", 14.0, theme::text_dim())))
    };
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .h(px(QUEUE_ROW_H))
        .px(px(6.0))
        .relative()
        .when(!lifted, |d| d.child(hover_fill(group, theme::card(), 8.0)))
        .child(handle)
        .child(art)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(song_title(tr.title.clone(), tr.explicit).text_size(theme::text_sm()))
                .child(div().truncate().text_size(theme::text_xs()).text_color(theme::text_dim()).child(tr.artist.clone())),
        )
}

impl Render for QueueDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        theme::on_panel(|| {
            queue_row_inner(self.from, &self.track, self.art.clone(), "q-lift".into(), true)
                .w(px(QUEUE_W - 12.0))
                .rounded(px(8.0))
                .bg(gpui::hsla(0.0, 0.0, 0.16, 1.0))
                .border_1()
                .border_color(gpui::hsla(0.0, 0.0, 1.0, 0.10))
                .font_family(theme::FONT)
                .text_color(theme::text())
                .cursor_grabbing()
        })
    }
}

/// `[ LABEL ]` section heading of the queue panel.
fn queue_label(text: String) -> Div {
    mono().px(px(16.0)).pt(px(4.0)).pb(px(8.0)).text_size(px(11.0)).text_color(theme::text_faint()).child(format!("[ {} ]", text.to_uppercase()))
}

/// Height of a section heading inside the queue's scroll area (drop slots
/// are computed from it).
const QUEUE_LABEL_H: f32 = 28.0;

/// `[ LABEL ]` heading between the queue's sections, fixed height.
fn queue_section(text: String) -> Div {
    mono()
        .flex_none()
        .h(px(QUEUE_LABEL_H))
        .px(px(6.0))
        .flex()
        .items_center()
        .text_size(px(11.0))
        .text_color(theme::text_faint())
        .child(div().truncate().child(format!("[ {} ]", text.to_uppercase())))
}

/// Upcoming index a drop at `y` (from the top of the list) lands on, with
/// `manual` hand-queued songs under their own heading above the rest.
fn queue_slot(y: f32, manual: usize, n: usize) -> usize {
    let mut y = y;
    if manual > 0 {
        y -= QUEUE_LABEL_H;
        let top = manual as f32 * QUEUE_ROW_H;
        if y < top {
            return ((y / QUEUE_ROW_H).floor().max(0.0) as usize).min(n.saturating_sub(1));
        }
        y -= top;
        if manual < n {
            y -= QUEUE_LABEL_H;
        }
        return (manual + (y / QUEUE_ROW_H).max(0.0).floor() as usize).min(n.saturating_sub(1));
    }
    if n > 0 {
        y -= QUEUE_LABEL_H;
    }
    ((y / QUEUE_ROW_H).floor().max(0.0) as usize).min(n.saturating_sub(1))
}

pub const QUEUE_W: f32 = 320.0;

pub fn queue_panel(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let up = app.upcoming();
    let n = up.len();
    let manual = app.manual_queue_len().min(n);
    let current = app.current.clone();
    let now = current.map(|tr| {
        let art = cover(app, &tr, 44.0, 6.0, cx);
        div()
            .mx(px(10.0))
            .mb(px(12.0))
            .p(px(6.0))
            .rounded(px(10.0))
            .bg(theme::card())
            .flex()
            .items_center()
            .gap(px(12.0))
            .child(art)
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(div().truncate().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).text_color(theme::pink()).child(tr.title))
                    .child(div().truncate().text_size(theme::text_sm()).text_color(theme::text_dim()).child(tr.artist)),
            )
    });
    let has_now = now.is_some();
    if !cx.has_active_drag() {
        app.queue_drop = None;
    }
    let drop = app.queue_drop;
    let slots = if drop.is_some() { n.saturating_sub(1) } else { n };
    let gaps = queue_gaps(slots, drop.map(|(_, at)| at));
    let mut rows: Vec<gpui::AnyElement> = Vec::new();
    let mut slot = 0;
    // Songs queued by hand ("Up next") come first under their own heading,
    // then what plays on from the list ("Next from: …").
    if manual > 0 {
        rows.push(queue_section(tf("queue.next_n", &[("n", &manual)]).to_string()).into_any_element());
    }
    for (i, tr) in up.into_iter().enumerate() {
        if i == manual && n > manual {
            let rest = n - manual;
            let head = match app.queue_from.as_deref() {
                Some(from) => tf("queue.from_n", &[("name", &from), ("n", &rest)]).to_string(),
                None if manual > 0 => tf("queue.later_n", &[("n", &rest)]).to_string(),
                None => tf("queue.next_n", &[("n", &rest)]).to_string(),
            };
            rows.push(queue_section(head).into_any_element());
        }
        if drop.is_some_and(|(from, _)| from == i) {
            continue;
        }
        let gap = gaps.get(slot).copied().unwrap_or(0.0);
        if gap > 0.5 {
            rows.push(div().flex_none().h(px(gap)).into_any_element());
        }
        slot += 1;
        let group: SharedString = format!("q-g-{i}").into();
        let art = app.cover_path(&tr, cx);
        let drag = QueueDrag { from: i, track: tr.clone(), art: art.clone() };
        let shown = if i < manual { i } else { i - manual };
        // In a jam: the picture of whoever added the song.
        let by = if i < manual { crate::ui::jam::added_by(app, &tr, i, cx) } else { None };
        rows.push(
            queue_row_inner(shown, &tr, art, group.clone(), false)
                .id(("q", i))
                .rounded(px(8.0))
                .cursor_pointer()
                .on_drag(drag, |d, _, _, cx| cx.new(|_| d.clone()))
                // A click plays this song and the queue goes on from it.
                .on_click(cx.listener(move |this, _, _, cx| this.queue_jump(i, cx)))
                .children(by)
                .child(
                    div()
                        .opacity(hover_k(&group))
                        .child(icon_button(("q-rm", i), "x", 14.0, theme::text_dim()).on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.queue_remove(i, cx)
                        }))),
                )
                .into_any_element(),
        );
    }
    // Dropped past the last row.
    let end = gaps.get(slot).copied().unwrap_or(0.0);
    if end > 0.5 {
        rows.push(div().flex_none().h(px(end)).into_any_element());
    }
    panel()
        .flex_none()
        .w(px(QUEUE_W))
        .h_full()
        .bg(theme::panel_strong())
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .pl(px(16.0))
                .pr(px(10.0))
                .pt(px(16.0))
                .pb(px(8.0))
                .child(div().flex_1().text_size(theme::text_lg()).font_weight(FontWeight::SEMIBOLD).child(t("queue.title")))
                .child(icon_button("q-close", "x", 16.0, theme::text_dim()).on_click(cx.listener(|this, _, _, cx| {
                    this.show_queue = false;
                    cx.notify();
                }))),
        )
        .when(has_now, |d| d.child(queue_label(t("queue.now").to_string())))
        .children(now)
        .when(n == 0, |d| d.child(queue_label(t("queue.next").to_string())))
        .child(fade_edges(div()
                .id("q-scroll")
                .track_scroll(&app.queue_scroll)
                .size_full()
                .overflow_y_scroll().touch_axis()
                .px(px(10.0))
                .when(!app.queue_ok, |d| d.child(div().px(px(6.0)).child(ascii_state(ART_OFFLINE, t("status.queue_offline"), ""))))
                .when(app.queue_ok && n == 0, |d| {
                    d.child(div().px(px(6.0)).child(ascii_state(ART_EMPTY, t("queue.empty"), if app.radio { t("queue.radio_next") } else { "".into() })))
                })
                // Dragging: the picked row leaves the list and a gap opens
                // where it would land.
                .on_drag_move::<QueueDrag>(cx.listener(move |this, e: &gpui::DragMoveEvent<QueueDrag>, _, cx| {
                    let from = e.drag(cx).from;
                    let y = e.event.position.y - e.bounds.top() - this.queue_scroll.offset().y;
                    let slot = queue_slot(f32::from(y), manual, n);
                    if this.queue_drop != Some((from, slot)) {
                        this.queue_drop = Some((from, slot));
                        cx.notify();
                    }
                }))
                .on_drop(cx.listener(|this, d: &QueueDrag, _, cx| {
                    let to = this.queue_drop.take().map_or(d.from, |(_, slot)| slot);
                    if to != d.from {
                        this.queue_move(d.from, to, cx)
                    }
                    cx.notify();
                }))
                .children(rows)
                , theme::panel_strong(), 18.0, Some(&app.queue_scroll)).flex_1().min_h_0().child(scrollbar(&app.queue_scroll, true, cx)))
        .child(
            div()
                .px(px(16.0))
                .pt(px(12.0))
                .pb(px(16.0))
                .border_t_1()
                .border_color(theme::border_faint())
                .child(radio_toggle(app, cx)),
        )
}

/// The queue's radio switch. Greyed out (with a hover explanation) when the
/// server has no working Last.fm key, so the switch never lies about what it
/// can do.
fn radio_toggle(app: &Glyph, cx: &mut Context<Glyph>) -> impl IntoElement {
    if app.radio_ok == Some(false) {
        return toggle_unavailable("q-radio", app.radio, t("queue.radio"), t("radio.off"));
    }
    toggle("q-radio", app.radio, t("queue.radio")).on_click(cx.listener(|this, _, _, cx| {
        this.radio = !this.radio;
        cx.notify();
    }))
}

// ------------------------------------------------------------------ add songs

/// "Add songs" panel next to an own playlist: search, or suggestions / liked
/// / recent songs, each with a + (a check once it is in the playlist).
pub fn add_songs_panel(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let Some(a) = app.add_songs.as_ref() else { return div().into_any_element() };
    let (id, tab, searching_q) = (a.id.clone(), a.tab, !a.query.trim().is_empty());
    let have: std::collections::HashSet<(glyph_protocol::ProviderId, String)> =
        app.library.playlist(&id).map(|p| p.tracks.iter().map(|t| (t.source, t.id.clone())).collect()).unwrap_or_default();
    let (songs, loading): (Vec<Track>, bool) = if searching_q {
        (a.results.clone(), a.searching)
    } else {
        match tab {
            1 => (app.library.liked.iter().take(100).cloned().collect(), false),
            2 => (app.library.recent_tracks().cloned().collect(), false),
            _ => (a.sugg.clone(), a.sugg_loading),
        }
    };
    let chip_tab = |id: &'static str, key: &'static str, n: u8, cx: &mut Context<Glyph>| {
        chip(t(key), tab == n).id(id).cursor_pointer().on_click(cx.listener(move |this, _, _, cx| this.set_add_tab(n, cx)))
    };
    let chips = div()
        .flex()
        .gap(px(6.0))
        .px(px(16.0))
        .pb(px(8.0))
        .child(chip_tab("add-t0", "add.tab.suggested", 0, cx))
        .child(chip_tab("add-t1", "add.tab.liked", 1, cx))
        .child(chip_tab("add-t2", "add.tab.recent", 2, cx));
    let body: gpui::AnyElement = if songs.is_empty() && loading {
        div().px(px(6.0)).child(skeleton_rows("add", 8)).into_any_element()
    } else if songs.is_empty() {
        div().px(px(6.0)).child(ascii_state(ART_EMPTY, t(if searching_q { "search.none.title" } else { "add.empty" }), "")).into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .children(songs.into_iter().enumerate().map(|(i, tr)| {
                let added = have.contains(&(tr.source, tr.id.clone()));
                let art = cover(app, &tr, 40.0, 4.0, cx);
                let (t_play, t_add) = (tr.clone(), tr.clone());
                div()
                    .id(("add-row", i))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .h(px(52.0))
                    .px(px(6.0))
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .relative()
                    .child(hover_fill(format!("add-row-{i}"), theme::card(), 8.0))
                    .on_click(cx.listener(move |this, _, _, cx| this.play_list(vec![t_play.clone()], 0, cx)))
                    .child(art)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(song_title(tr.title.clone(), tr.explicit).text_size(theme::text_sm()))
                            .child(div().truncate().text_size(theme::text_xs()).text_color(theme::text_dim()).child(tr.artist.clone())),
                    )
                    .child(
                        icon_button(("add-btn", i), if added { "circle-check" } else { "circle-plus" }, 18.0, if added { theme::pink() } else { theme::text_dim() })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.add_songs_toggle(t_add.clone(), cx)
                            })),
                    )
                    .into_any_element()
            }))
            .into_any_element()
    };
    panel()
        .flex_none()
        .w(px(QUEUE_W))
        .h_full()
        .bg(theme::panel_strong())
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .pl(px(16.0))
                .pr(px(10.0))
                .pt(px(20.0))
                .pb(px(12.0))
                .child(div().flex_1().text_size(theme::text_lg()).font_weight(FontWeight::SEMIBOLD).child(t("add.title")))
                .child(icon_button("add-close", "x", 16.0, theme::text_dim()).on_click(cx.listener(|this, _, _, cx| this.close_add_songs(cx)))),
        )
        .child(
            div()
                .mx(px(16.0))
                .mb(px(10.0))
                .h(px(36.0))
                .px(px(12.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .rounded_full()
                .bg(theme::card())
                .child(icon("search", 14.0, theme::text_dim()))
                .child(div().flex_1().min_w_0().text_size(theme::text_sm()).child(app.add_input.clone())),
        )
        .when(!searching_q, |d| d.child(chips))
        .child(
            fade_edges(
                div().id("add-scroll").track_scroll(&app.add_scroll).size_full().overflow_y_scroll().touch_axis().px(px(10.0)).pb(px(10.0)).child(body),
                theme::panel_strong(),
                18.0,
                Some(&app.add_scroll),
            )
            .flex_1()
            .min_h_0()
            .child(scrollbar(&app.add_scroll, true, cx)),
        )
        .into_any_element()
}
