//! Main-area views: home, search, library, queue panel.

use crate::app::{BackendState, Glyph, LibTab, SearchFilter, View};
use crate::theme;
use crate::ui::widgets::*;
use crate::visuals::Rgb;
use glyph_core::BackendMode;
use glyph_protocol::Track;
use gpui::{div, prelude::*, px, Context, FontWeight, Window};
use std::rc::Rc;

pub const LOGO: &str = r"         __            __
  ___ _ / /__ __ ___  / /
 / _ `// // // // _ \/ _ \
 \_, //_/ \_, // .__/_//_/
/___/     /___//_/";

fn accent(app: &Glyph) -> Rgb {
    app.palette.map(|p| p.accent()).unwrap_or(Rgb::hex(0xff5fd2))
}

fn scroll_area(id: &'static str) -> gpui::Stateful<gpui::Div> {
    div().id(id).size_full().overflow_y_scroll().px(px(28.0)).py(px(24.0))
}

fn track_list(app: &Glyph, tracks: Vec<Track>, prefix: &'static str, cx: &mut Context<Glyph>) -> gpui::Div {
    let list = Rc::new(tracks);
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .children((0..list.len()).map(|i| track_row(app, list.clone(), i, prefix, cx)))
}

// ------------------------------------------------------------------ home

pub fn home(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let status = match &app.backend_state {
        BackendState::Connecting => format!("{} GET /v1/health ...", app.spinner()),
        BackendState::Ready(BackendMode::Server { url, server_version }) => {
            format!("GET {url}/v1/health -> 200 · server v{server_version}")
        }
        BackendState::Ready(BackendMode::Direct) => "MODE direct -> provider-apis mit lokalen keys".into(),
        BackendState::Ready(BackendMode::Incompatible { url, protocol_version }) => {
            format!("GET {url}/v1/health -> protokoll v{protocol_version} · INKOMPATIBEL")
        }
        BackendState::Offline(e) => format!("BACKEND -> offline ({e})"),
    };
    let recent: Vec<Track> = app.library.recent.iter().take(8).cloned().collect();
    let liked_n = app.library.liked.len();
    let a = accent(app);

    scroll_area("home-scroll")
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(22.0))
                .pt(px(24.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .text_size(px(12.0))
                        .line_height(px(13.0))
                        .text_color(a.lighten(0.45).hsla(0.9))
                        .children(LOGO.lines().map(|l| div().whitespace_nowrap().child(l.to_string()))),
                )
                .child(
                    div()
                        .text_color(theme::text_dim())
                        .max_w(px(560.0))
                        .child("suche überall. spiele nur aus legalen, freien quellen. keine telemetrie, keine konten, alles lokal."),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(10.0))
                        .child(
                            outline_button("home-search", "[ / SUCHEN ]")
                                .on_click(cx.listener(|this, _, w, cx| this.navigate(View::Search, w, cx))),
                        )
                        .child(
                            outline_button("home-play", "[ ▶ WEITERHÖREN ]")
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_play(cx))),
                        )
                        .child(
                            outline_button("home-liked", format!("[ ♥ {liked_n} ]"))
                                .on_click(cx.listener(|this, _, w, cx| {
                                    this.lib_tab = LibTab::Liked;
                                    this.navigate(View::Library, w, cx)
                                })),
                        ),
                )
                .child(
                    div()
                        .text_size(theme::text_xs())
                        .text_color(theme::text_faint())
                        .child(status),
                )
                .child(home_cards(app, cx))
                .child(section_title("♪", "zuletzt gehört"))
                .child(if recent.is_empty() {
                    ascii_state(ART_EMPTY, "noch nichts gehört", "ctrl+k und los").into_any_element()
                } else {
                    cover_row(recent, "home-row", cx).into_any_element()
                }),
        )
}

// ------------------------------------------------------------------ search

pub fn search(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let a = accent(app);
    let s = &app.search;
    let filter = s.filter.unwrap_or(SearchFilter::All);
    let chip_btn = |id: &'static str, label: &'static str, f: SearchFilter, cx: &mut Context<Glyph>| {
        let on = filter == f;
        div()
            .id(id)
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.search.filter = Some(f);
                cx.notify();
            }))
            .child(chip(label, a, on))
    };
    let head = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(tag("suche"))
        .child(div().flex_1())
        .child(chip_btn("f-all", "alle", SearchFilter::All, cx))
        .child(chip_btn("f-tracks", "track", SearchFilter::Tracks, cx))
        .child(chip_btn("f-albums", "album", SearchFilter::Albums, cx))
        .child(chip_btn("f-artists", "artist", SearchFilter::Artists, cx));

    let s = &app.search;
    let body: gpui::AnyElement = if let Some(e) = &s.error {
        ascii_state(ART_OFFLINE, e.clone(), "einstellungen → server / keys prüfen").into_any_element()
    } else if s.query.trim().is_empty() {
        ascii_state(ART_SEARCH, "tippe, um zu suchen", "audius · jamendo · archive · soundcloud · youtube · ...")
            .into_any_element()
    } else if s.results.is_empty() && s.loading {
        div().py(px(24.0)).child(spinner_line(app.spinner(), format!("GET /v1/search?q={}", s.query))).into_any_element()
    } else if s.results.is_empty() {
        ascii_state(ART_EMPTY, "nichts gefunden", format!("\"{}\"", s.query)).into_any_element()
    } else {
        match filter {
            SearchFilter::All | SearchFilter::Tracks => {
                track_list(app, app.search.results.clone(), "search-row", cx).into_any_element()
            }
            SearchFilter::Albums | SearchFilter::Artists => {
                let mut groups: Vec<(String, String, Vec<Track>)> = Vec::new();
                for t in &app.search.results {
                    let (k, sub) = if filter == SearchFilter::Albums {
                        match &t.album {
                            Some(al) => (al.clone(), t.artist.clone()),
                            None => continue,
                        }
                    } else {
                        (t.artist.clone(), String::new())
                    };
                    match groups.iter_mut().find(|g| g.0 == k) {
                        Some(g) => g.2.push(t.clone()),
                        None => groups.push((k, sub, vec![t.clone()])),
                    }
                }
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .children(groups.into_iter().enumerate().map(|(i, (name, sub, tracks))| {
                        let n = tracks.len();
                        div()
                            .id(("group", i))
                            .flex()
                            .items_center()
                            .gap(px(12.0))
                            .px(px(10.0))
                            .py(px(8.0))
                            .rounded(px(8.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::hover()))
                            .on_click(cx.listener(move |this, _, _, cx| this.play_list(tracks.clone(), 0, cx)))
                            .child(chip(if filter == SearchFilter::Albums { "album" } else { "artist" }, a, true))
                            .child(div().flex_1().truncate().text_color(theme::text()).child(name))
                            .child(div().text_size(theme::text_sm()).text_color(theme::text_faint()).child(sub))
                            .child(div().text_size(theme::text_sm()).text_color(theme::text_faint()).child(format!("{n} tracks")))
                    }))
                    .into_any_element()
            }
        }
    };
    let s = &app.search;
    let footer = div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .text_size(theme::text_xs())
        .text_color(theme::text_faint())
        .when(s.loading && !s.results.is_empty(), |d| {
            d.child(spinner_line(app.spinner(), "weitere quellen antworten ..."))
        })
        .children(s.errors.iter().map(|(p, m)| {
            div().text_color(theme::warn().opacity(0.8)).child(format!("[{}] {} -> {}", p.badge(), p.display_name(), m))
        }));

    scroll_area("search-scroll").child(
        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(head)
            .child(
                div()
                    .text_size(px(22.0))
                    .text_color(theme::text())
                    .child(if s.query.is_empty() { "suche".to_string() } else { format!("\"{}\"", s.query) }),
            )
            .child(body)
            .child(footer),
    )
}

// ------------------------------------------------------------------ library

pub fn library(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let a = accent(app);
    let tab = |id: &'static str, label: String, t: LibTab, app: &Glyph, cx: &mut Context<Glyph>| {
        let on = app.lib_tab == t;
        div()
            .id(id)
            .cursor_pointer()
            .pb(px(4.0))
            .border_b_1()
            .border_color(if on { a.lighten(0.3).hsla(0.9) } else { gpui::transparent_black() })
            .text_color(if on { theme::text() } else { theme::text_faint() })
            .hover(|s| s.text_color(theme::text()))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.lib_tab = t;
                this.open_playlist = None;
                cx.notify();
            }))
            .child(label)
    };
    let lib = &app.library;
    let tabs = div()
        .flex()
        .gap(px(22.0))
        .child(tab("t-liked", format!("liked {}", lib.liked.len()), LibTab::Liked, app, cx))
        .child(tab("t-recent", format!("zuletzt {}", lib.recent.len()), LibTab::Recent, app, cx))
        .child(tab("t-pl", format!("playlists {}", lib.playlists.len()), LibTab::Playlists, app, cx));

    let offline = !app.library_ok;
    let body: gpui::AnyElement = match app.lib_tab {
        _ if offline => ascii_state(ART_OFFLINE, "[ LIBRARY OFFLINE ]", "library.json konnte nicht geladen werden").into_any_element(),
        LibTab::Liked if app.library.liked.is_empty() => {
            ascii_state(ART_EMPTY, "noch keine likes", "♡ neben einem song").into_any_element()
        }
        LibTab::Liked => track_list(app, app.library.liked.clone(), "lib-liked", cx).into_any_element(),
        LibTab::Recent if app.library.recent.is_empty() => {
            ascii_state(ART_EMPTY, "noch nichts gehört", "die letzten 50 songs landen hier").into_any_element()
        }
        LibTab::Recent => track_list(app, app.library.recent.clone(), "lib-recent", cx).into_any_element(),
        LibTab::Playlists => {
            if let Some(p) = app
                .open_playlist
                .as_ref()
                .and_then(|id| app.library.playlists.iter().find(|p| &p.id == id))
                .cloned()
            {
                let id = p.id.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .child(ascii_button("pl-back", "<- zurück").on_click(cx.listener(|this, _, _, cx| {
                                this.open_playlist = None;
                                cx.notify();
                            })))
                            .child(div().flex_1().text_size(px(20.0)).child(p.name.clone()))
                            .when(!p.tracks.is_empty(), |d| {
                                let tracks = p.tracks.clone();
                                d.child(outline_button("pl-play", "[ ▶ PLAY ]").on_click(cx.listener(move |this, _, _, cx| this.play_list(tracks.clone(), 0, cx))))
                            })
                            .child(ascii_button("pl-del", "löschen").on_click(cx.listener(move |this, _, _, cx| this.delete_playlist(&id, cx)))),
                    )
                    .when_some(p.origin.clone(), |d, o| d.child(div().text_size(theme::text_xs()).text_color(theme::text_faint()).child(format!("IMPORT {o}"))))
                    .child(if p.tracks.is_empty() {
                        ascii_state(ART_EMPTY, "leer", "aktuellen song über \"+ aktuellen song\" hinzufügen").into_any_element()
                    } else {
                        track_list(app, p.tracks.clone(), "pl-row", cx).into_any_element()
                    })
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(div().flex().child(outline_button("pl-new", "[ + NEUE PLAYLIST ]").on_click(cx.listener(|this, _, _, cx| this.create_playlist(cx)))))
                    .children(app.library.playlists.iter().enumerate().map(|(i, p)| {
                        let id = p.id.clone();
                        let id2 = p.id.clone();
                        let cur = app.current.clone();
                        div()
                            .id(("pl", i))
                            .flex()
                            .items_center()
                            .gap(px(12.0))
                            .px(px(10.0))
                            .py(px(8.0))
                            .rounded(px(8.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::hover()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_playlist = Some(id.clone());
                                cx.notify();
                            }))
                            .child(div().text_color(theme::text_faint()).child("≡"))
                            .child(div().flex_1().truncate().child(p.name.clone()))
                            .child(div().text_size(theme::text_sm()).text_color(theme::text_faint()).child(format!("{} tracks", p.tracks.len())))
                            .when_some(cur, |d, t| {
                                d.child(ascii_button(("pl-add", i), "+ aktuellen song").text_size(theme::text_sm()).on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.add_to_playlist(&id2, t.clone(), cx)
                                })))
                            })
                    }))
                    .into_any_element()
            }
        }
    };

    scroll_area("lib-scroll").child(
        div()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .child(tag("library"))
            .child(heading("bibliothek"))
            .child(tabs)
            .child(body),
    )
}

// ------------------------------------------------------------------ queue

pub fn queue_panel(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let up = app.upcoming();
    let n = up.len();
    panel()
        .flex_none()
        .w(px(300.0))
        .h_full()
        .bg(theme::panel_strong())
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .px(px(14.0))
                .py(px(12.0))
                .child(tag("queue"))
                .child(div().flex_1())
                .child(div().text_size(theme::text_xs()).text_color(theme::text_faint()).child(format!("{n} als nächstes")))
                .child(ascii_button("q-close", "×").on_click(cx.listener(|this, _, _, cx| {
                    this.show_queue = false;
                    cx.notify();
                }))),
        )
        .when_some(app.current.clone(), |d, t| {
            d.child(
                div()
                    .px(px(14.0))
                    .pb(px(10.0))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(div().text_size(theme::text_xs()).text_color(theme::text_faint()).child("jetzt"))
                    .child(div().truncate().text_color(theme::text()).child(format!("▶ {}", t.title)))
                    .child(div().truncate().text_size(theme::text_sm()).text_color(theme::text_dim()).child(t.artist)),
            )
        })
        .child(
            div()
                .id("q-scroll")
                .flex_1()
                .overflow_y_scroll()
                .px(px(6.0))
                .when(!app.queue_ok, |d| d.child(ascii_state(ART_OFFLINE, "[ QUEUE OFFLINE ]", "")))
                .when(app.queue_ok && n == 0, |d| {
                    d.child(ascii_state(ART_EMPTY, "queue leer", if app.radio { "radio übernimmt danach" } else { "" }))
                })
                .children(up.into_iter().enumerate().map(|(i, t)| {
                    div()
                        .id(("q", i))
                        .group("q")
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(8.0))
                        .py(px(6.0))
                        .rounded(px(6.0))
                        .hover(|s| s.bg(theme::hover()))
                        .child(div().w(px(18.0)).text_size(theme::text_xs()).text_color(theme::text_ghost()).child(format!("{:02}", i + 1)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(div().truncate().text_size(theme::text_sm()).child(t.title.clone()))
                                .child(div().truncate().text_size(theme::text_xs()).text_color(theme::text_faint()).child(t.artist.clone())),
                        )
                        .child(ascii_button(("q-up", i), "↑").on_click(cx.listener(move |this, _, _, cx| {
                            if i > 0 {
                                this.queue_move(i, i - 1, cx)
                            }
                        })))
                        .child(ascii_button(("q-down", i), "↓").on_click(cx.listener(move |this, _, _, cx| {
                            if i + 1 < n {
                                this.queue_move(i, i + 1, cx)
                            }
                        })))
                        .child(ascii_button(("q-rm", i), "×").on_click(cx.listener(move |this, _, _, cx| this.queue_remove(i, cx))))
                }))
        )
        .child(
            div()
                .px(px(14.0))
                .py(px(10.0))
                .border_t_1()
                .border_color(theme::border_faint())
                .flex()
                .items_center()
                .gap(px(8.0))
                .text_size(theme::text_sm())
                .child(div().flex_1().text_color(theme::text_faint()).child("radio wenn leer"))
                .child(
                    ascii_button("q-radio", if app.radio { "[x]" } else { "[ ]" })
                        .font_weight(FontWeight::BOLD)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.radio = !this.radio;
                            cx.notify();
                        })),
                ),
        )
}

fn section_title(icon: &'static str, title: &'static str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .pt(px(8.0))
        .child(div().text_color(theme::text_dim()).child(icon))
        .child(div().text_size(px(18.0)).font_weight(FontWeight::MEDIUM).child(title))
}

/// Flat dark cards (#141414-like), 56 px cover left, title + subtitle.
fn home_cards(app: &Glyph, cx: &mut Context<Glyph>) -> gpui::Div {
    let liked = app.library.liked.clone();
    let n = liked.len();
    let card = |id: gpui::ElementId| {
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(px(12.0))
            .h(px(56.0))
            .w(px(300.0))
            .rounded(px(6.0))
            .overflow_hidden()
            .bg(gpui::hsla(0.0, 0.0, 0.08, 0.72))
            .cursor_pointer()
            .hover(|s| s.bg(gpui::hsla(0.0, 0.0, 0.12, 0.8)))
    };
    let mut cards = vec![card("card-liked".into())
        .on_click(cx.listener(move |this, _, w, cx| {
            if !liked.is_empty() {
                this.play_list(liked.clone(), 0, cx)
            } else {
                this.lib_tab = LibTab::Liked;
                this.navigate(View::Library, w, cx)
            }
        }))
        .child(div().size(px(56.0)).flex().items_center().justify_center().bg(gpui::hsla(0.0, 0.0, 0.14, 1.0)).child("♥"))
        .child(div().flex().flex_col().child("liked songs").child(div().text_size(theme::text_sm()).text_color(theme::text_faint()).child(format!("{n} songs"))))
        .into_any_element()];
    let list = Rc::new(app.library.recent.iter().take(5).cloned().collect::<Vec<_>>());
    for i in 0..list.len() {
        let t = list[i].clone();
        let l = list.clone();
        cards.push(
            card(("card", i).into())
                .on_click(cx.listener(move |this, _, _, cx| this.play_list((*l).clone(), i, cx)))
                .child(mini_cover(&t, 56.0).rounded(px(0.0)).border_0())
                .child(
                    div().flex().flex_col().min_w_0()
                        .child(div().truncate().child(t.title.clone()))
                        .child(div().truncate().text_size(theme::text_sm()).text_color(theme::text_faint()).child(t.artist.clone())),
                )
                .into_any_element(),
        );
    }
    div().flex().flex_wrap().gap(px(10.0)).children(cards)
}

/// Horizontal row of square ASCII covers (176 px) with title/artist below.
fn cover_row(tracks: Vec<Track>, prefix: &'static str, cx: &mut Context<Glyph>) -> gpui::Stateful<gpui::Div> {
    let list = Rc::new(tracks);
    div()
        .id(prefix)
        .flex()
        .gap(px(18.0))
        .overflow_x_scroll()
        .children((0..list.len()).map(|i| {
            let t = list[i].clone();
            let l = list.clone();
            div()
                .id((prefix, i))
                .flex_none()
                .w(px(176.0))
                .flex()
                .flex_col()
                .gap(px(6.0))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.play_list((*l).clone(), i, cx)))
                .child(big_cover(&t, 176.0))
                .child(div().truncate().child(t.title.clone()))
                .child(div().truncate().text_size(theme::text_sm()).text_color(theme::text_faint()).child(t.artist.clone()))
        }))
}

fn big_cover(t: &Track, size: f32) -> gpui::Div {
    let seed = format!("{}{}", t.artist, t.title);
    let pal = crate::visuals::Palette::from_seed(&seed);
    let rows = crate::visuals::ascii::pattern(&seed, 26, 16);
    div()
        .size(px(size))
        .rounded(px(8.0))
        .overflow_hidden()
        .bg(pal.0[0].scale(0.3).hsla(1.0))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .text_size(px(10.0))
        .line_height(px(10.5))
        .text_color(pal.0[2].lighten(0.3).hsla(0.9))
        .children(rows.into_iter().map(|(l, _)| div().whitespace_nowrap().child(l)))
}
