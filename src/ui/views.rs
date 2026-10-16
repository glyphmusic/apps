//! Main-area views: home, search, library, queue panel. Content floats
//! directly on the background; only individual cards are flat surfaces.

use crate::app::{BackendState, Glyph, LibTab, SearchFilter, View};
use crate::i18n::{t, tf};
use crate::theme;
use crate::ui::widgets::*;
use glyph_core::BackendMode;
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

    scroll_area("home-scroll").child(
        div()
            .flex()
            .flex_col()
            .gap(px(24.0))
            .child(heading(greeting()))
            .child(cards)
            .child(section_title("disc-3", t("home.recent")))
            .child(row),
    )
}

/// Flat dark cards, 56 px cover left, title + subtitle.
fn home_cards(app: &mut Glyph, cx: &mut Context<Glyph>) -> gpui::Div {
    let liked = app.library.liked.clone();
    let n = liked.len();
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
    let mut cards = vec![base("card-liked".into())
        .on_click(cx.listener(move |this, _, w, cx| {
            if liked.is_empty() {
                this.lib_tab = LibTab::Liked;
                this.navigate(View::Library, w, cx)
            } else {
                this.play_list(liked.clone(), 0, cx)
            }
        }))
        .child(
            div()
                .size(px(56.0))
                .flex()
                .items_center()
                .justify_center()
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
    let list = Rc::new(app.library.recent.iter().take(5).cloned().collect::<Vec<_>>());
    for i in 0..list.len() {
        let tr = list[i].clone();
        let l = list.clone();
        let art = cover(app, &tr, 56.0, 0.0, cx);
        cards.push(
            base(("card", i).into())
                .on_click(cx.listener(move |this, _, _, cx| this.play_list((*l).clone(), i, cx)))
                .child(art)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .gap(px(2.0))
                        .child(div().truncate().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).child(tr.title.clone()))
                        .child(div().truncate().text_size(theme::text_sm()).text_color(theme::text_dim()).child(tr.artist.clone())),
                )
                .into_any_element(),
        );
    }
    div().flex().flex_wrap().gap(px(10.0)).children(cards)
}

/// Horizontal row of square covers (176 px) with title / artist below.
fn cover_row(app: &mut Glyph, tracks: Vec<Track>, prefix: &'static str, cx: &mut Context<Glyph>) -> gpui::Stateful<gpui::Div> {
    let list = Rc::new(tracks);
    let items: Vec<_> = (0..list.len())
        .map(|i| {
            let tr = list[i].clone();
            let l = list.clone();
            let art = cover(app, &tr, 176.0, 8.0, cx);
            div()
                .id((prefix, i))
                .flex_none()
                .w(px(176.0))
                .flex()
                .flex_col()
                .gap(px(4.0))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.play_list((*l).clone(), i, cx)))
                .child(art)
                .child(div().pt(px(4.0)).truncate().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).child(tr.title.clone()))
                .child(div().truncate().text_size(theme::text_sm()).text_color(theme::text_dim()).child(tr.artist.clone()))
        })
        .collect();
    div().id(prefix).flex().gap(px(20.0)).overflow_x_scroll().children(items)
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
        skeleton_rows("search", 8).pt(px(8.0)).into_any_element()
    } else if app.search.results.is_empty() {
        ascii_state(ART_EMPTY, t("search.none.title"), format!("\u{201c}{query}\u{201d}")).into_any_element()
    } else {
        match filter {
            SearchFilter::All | SearchFilter::Tracks => {
                let r = app.search.results.clone();
                track_list(app, r, "search-row", cx).into_any_element()
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
                        let art = cover(app, &tracks[0], 44.0, if filter == SearchFilter::Artists { 22.0 } else { 6.0 }, cx);
                        div()
                            .id(("group", i))
                            .flex()
                            .items_center()
                            .gap(px(14.0))
                            .px(px(10.0))
                            .py(px(6.0))
                            .rounded(px(8.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::card()))
                            .on_click(cx.listener(move |this, _, _, cx| this.play_list(tracks.clone(), 0, cx)))
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
    let s = &app.search;
    let footer = div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .when(s.loading && !s.results.is_empty(), |d| d.child(spinner_line(app.spinner(), t("search.more"))))
        .children(s.errors.iter().map(|(p, m)| status_line(format!("[{}] {} -> {}", p.badge(), p.display_name(), m))));

    scroll_area("search-scroll").child(
        div()
            .flex()
            .flex_col()
            .gap(px(18.0))
            .child(heading(if query.trim().is_empty() { t("nav.search") } else { format!("\u{201c}{query}\u{201d}").into() }))
            .child(chips)
            .child(body)
            .child(footer),
    )
}

// ------------------------------------------------------------------ library

pub fn library(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let tab = |id: &'static str, label: gpui::SharedString, tb: LibTab, app: &Glyph, cx: &mut Context<Glyph>| {
        div()
            .id(id)
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.lib_tab = tb;
                this.open_playlist = None;
                cx.notify();
            }))
            .child(chip(label, app.lib_tab == tb))
    };
    let (nl, nr, np) = (app.library.liked.len(), app.library.recent.len(), app.library.playlists.len());
    let tabs = div()
        .flex()
        .gap(px(8.0))
        .child(tab("t-liked", tf("library.tab.liked", &[("n", &nl)]), LibTab::Liked, app, cx))
        .child(tab("t-recent", tf("library.tab.recent", &[("n", &nr)]), LibTab::Recent, app, cx))
        .child(tab("t-pl", tf("library.tab.playlists", &[("n", &np)]), LibTab::Playlists, app, cx));

    let body: gpui::AnyElement = if !app.library_ok {
        ascii_state(ART_OFFLINE, t("status.library_offline"), t("library.offline.sub")).into_any_element()
    } else {
        match app.lib_tab {
            LibTab::Liked if app.library.liked.is_empty() => {
                ascii_state(ART_EMPTY, t("library.liked.empty"), t("library.liked.empty.sub")).into_any_element()
            }
            LibTab::Liked => {
                let l = app.library.liked.clone();
                track_list(app, l, "lib-liked", cx).into_any_element()
            }
            LibTab::Recent if app.library.recent.is_empty() => {
                ascii_state(ART_EMPTY, t("home.empty.title"), t("library.recent.empty.sub")).into_any_element()
            }
            LibTab::Recent => {
                let l = app.library.recent.clone();
                track_list(app, l, "lib-recent", cx).into_any_element()
            }
            LibTab::Playlists => playlists(app, cx),
        }
    };

    scroll_area("lib-scroll").child(
        div()
            .flex()
            .flex_col()
            .gap(px(18.0))
            .child(heading(t("nav.library")))
            .child(tabs)
            .child(body),
    )
}

fn playlists(app: &mut Glyph, cx: &mut Context<Glyph>) -> gpui::AnyElement {
    let open = app
        .open_playlist
        .as_ref()
        .and_then(|id| app.library.playlists.iter().find(|p| &p.id == id))
        .cloned();
    if let Some(p) = open {
        let id = p.id.clone();
        let tracks = p.tracks.clone();
        let list = if p.tracks.is_empty() {
            ascii_state(ART_EMPTY, t("library.playlist.empty"), t("library.playlist.empty.sub")).into_any_element()
        } else {
            track_list(app, p.tracks.clone(), "pl-row", cx).into_any_element()
        };
        return div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(icon_button("pl-back", "chevron-left", 18.0, theme::text()).on_click(cx.listener(|this, _, _, cx| {
                        this.open_playlist = None;
                        cx.notify();
                    })))
                    .child(div().flex_1().text_size(px(18.0)).font_weight(FontWeight::SEMIBOLD).child(p.name.clone()))
                    .when(!tracks.is_empty(), |d| {
                        d.child(primary_button("pl-play", t("common.play")).on_click(cx.listener(move |this, _, _, cx| this.play_list(tracks.clone(), 0, cx))))
                    })
                    .child(icon_button("pl-del", "trash-2", 16.0, theme::text_dim()).on_click(cx.listener(move |this, _, _, cx| this.delete_playlist(&id, cx)))),
            )
            .when_some(p.origin.clone(), |d, o| d.child(status_line(format!("IMPORT {o}"))))
            .child(list)
            .into_any_element();
    }
    let cur = app.current.clone();
    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(div().flex().pb(px(8.0)).child(button("pl-new", t("library.playlist.new")).on_click(cx.listener(|this, _, _, cx| this.create_playlist(cx)))))
        .children(app.library.playlists.iter().enumerate().map(|(i, p)| {
            let id = p.id.clone();
            let id2 = p.id.clone();
            let cur = cur.clone();
            div()
                .id(("pl", i))
                .flex()
                .items_center()
                .gap(px(14.0))
                .px(px(10.0))
                .py(px(8.0))
                .rounded(px(8.0))
                .cursor_pointer()
                .hover(|s| s.bg(theme::card()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.open_playlist = Some(id.clone());
                    cx.notify();
                }))
                .child(div().size(px(44.0)).rounded(px(6.0)).bg(theme::card()).flex().items_center().justify_center().child(icon("list-music", 18.0, theme::text_dim())))
                .child(div().flex_1().truncate().text_size(theme::text_md()).child(p.name.clone()))
                .child(div().text_size(theme::text_sm()).text_color(theme::text_dim()).child(tf("common.n_songs", &[("n", &p.tracks.len())])))
                .when_some(cur, |d, tr| {
                    d.child(text_button(("pl-add", i), t("library.playlist.add_current")).text_size(theme::text_sm()).on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.add_to_playlist(&id2, tr.clone(), cx)
                    })))
                })
        }))
        .into_any_element()
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
        .child(
            div()
                .id("q-scroll")
                .flex_1()
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
                                .child(div().truncate().text_size(theme::text_sm()).child(tr.title.clone()))
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
                })),
        )
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
