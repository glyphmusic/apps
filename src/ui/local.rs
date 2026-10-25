//! "Local files": the music folders on this computer by folder, artist,
//! album or as one list of songs. Files play where they are; nothing is
//! copied or uploaded (see `glyph_core::local_library`).

use crate::app::{Glyph, LOCAL_ALBUMS, LOCAL_ARTISTS, LOCAL_FOLDERS, LOCAL_SONGS};
use crate::i18n::{t, tf};
use crate::theme;
use crate::ui::lists::{back_button, round_play, song_row};
use crate::ui::widgets::*;
use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, Hsla, MouseButton, SharedString, Stateful, Window};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Row pitch (same as the song lists).
const PITCH: f32 = 56.0;

fn white(a: f32) -> Hsla {
    gpui::hsla(0.0, 0.0, 1.0, a)
}

/// Folder name as shown ("Music"; a drive root keeps its whole path).
fn display_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string_lossy().into_owned())
}

/// Bordered header action (same as the library's "Statistics" button).
fn action(id: &'static str, icon_name: &'static str, label: SharedString) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(6.0))
        .h(px(32.0))
        .px(px(14.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(theme::border())
        .cursor_pointer()
        .text_size(theme::text_sm())
        .text_color(white(0.9))
        .hover(|s| s.bg(theme::card_hover()))
        .child(icon(icon_name, 14.0, white(0.9)))
        .child(label)
}

/// Folder / artist / album row: picture or icon, name, detail line, chevron.
fn entry_row(id: (&'static str, usize), art: AnyElement, name: String, sub: String) -> Stateful<gpui::Div> {
    let key = format!("{}-{}", id.0, id.1);
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(14.0))
        .h(px(PITCH))
        .px(px(10.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .relative()
        .child(hover_fill(key, white(0.05), 8.0))
        .child(art)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.0))
                .child(div().truncate().text_size(theme::text_md()).text_color(theme::text()).child(name))
                .child(div().truncate().text_size(theme::text_sm()).text_color(theme::text_dim()).child(sub)),
        )
        .child(icon("chevron-right", 16.0, theme::text_faint()))
}

/// Square icon in place of a cover.
fn icon_art(name: &'static str) -> AnyElement {
    div()
        .flex_none()
        .size(px(40.0))
        .rounded(px(6.0))
        .bg(theme::text().opacity(0.08))
        .flex()
        .items_center()
        .justify_center()
        .child(icon(name, 18.0, theme::text_dim()))
        .into_any_element()
}

fn n_songs(n: usize) -> String {
    tf("common.n_songs", &[("n", &n)]).to_string()
}

pub fn view(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    let back = back_button(app, cx);
    let page = app.local_page();
    let roots = app.local_folders();
    let lib = app.backend.as_ref().map(|b| b.local_library());
    let total = lib.as_ref().map_or(0, |l| l.len());
    let progress = lib.as_ref().and_then(|l| l.scan_progress());
    let tab = app.local.tab;

    // Title, counts, actions.
    let info: SharedString = match progress {
        Some((done, all)) => tf("local.scanning", &[("done", &done), ("total", &all)]),
        None if app.local.scanning => t("local.scanning_start"),
        None => tf("local.info", &[("n", &total), ("folders", &roots.len())]),
    };
    let has_songs = total > 0;
    // One of the listed songs is on: the button pauses / resumes it.
    let here = app.current.as_ref().is_some_and(|c| c.id.starts_with(glyph_core::local_library::ID_PREFIX));
    let (playing, loading) = (here && app.is_playing(), here && app.is_loading());
    let head = div()
        .flex()
        .items_center()
        .justify_between()
        .flex_wrap()
        .gap(px(12.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(14.0))
                .when(has_songs, |d| {
                    d.child(round_play("local-play", playing, loading, 44.0).on_click(cx.listener(move |this, _, _, cx| {
                        if here {
                            this.toggle_play(cx)
                        } else {
                            this.play_local(false, cx)
                        }
                    })))
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(heading(t("local.title")))
                        .child(mono().text_size(px(11.5)).text_color(theme::text_faint()).child(format!("[ {} ]", info.to_uppercase()))),
                ),
        )
        .child(
            div()
                .flex()
                .gap(px(8.0))
                .when(has_songs, |d| d.child(action("local-shuffle", "shuffle", t("local.shuffle")).on_click(cx.listener(|this, _, _, cx| this.play_local(true, cx)))))
                .when(!roots.is_empty(), |d| {
                    d.child(action("local-rescan", "refresh-cw", t("local.rescan")).when(app.local.scanning, |d| d.opacity(0.5)).on_click(cx.listener(|this, _, _, cx| this.rescan_local(cx))))
                })
                .child(action("local-add", "folder-plus", t("local.add")).on_click(cx.listener(|this, _, _, cx| this.add_local_folder(cx)))),
        );

    // Tabs and the filter.
    let tabs = div()
        .flex()
        .items_center()
        .justify_between()
        .flex_wrap()
        .gap(px(10.0))
        .child(div().flex().gap(px(8.0)).children(
            [(LOCAL_FOLDERS, t("local.tab.folders")), (LOCAL_ARTISTS, t("local.tab.artists")), (LOCAL_ALBUMS, t("local.tab.albums")), (LOCAL_SONGS, t("local.tab.songs"))]
                .into_iter()
                .map(|(k, label)| {
                    div()
                        .id(("local-tab", k as usize))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| this.set_local_tab(k, cx)))
                        .child(chip(label, k == tab))
                }),
        ))
        .when(has_songs, |d| {
            d.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .w(px(240.0))
                    .h(px(32.0))
                    .px(px(10.0))
                    .rounded(px(8.0))
                    .bg(gpui::hsla(0.0, 0.0, 0.08, 0.92))
                    .border_1()
                    .border_color(white(0.08))
                    .text_size(theme::text_sm())
                    .child(icon("search", 14.0, white(0.5)))
                    .child(div().flex_1().min_w_0().child(app.local_search.clone())),
            )
        });

    // Where in the folders / artists / albums the page is.
    let crumbs = crumbs(app, &roots, cx);
    let body = body(app, &page, &roots, cx);
    div()
        .size_full()
        .flex()
        .flex_col()
        .gap(px(16.0))
        .px(px(theme::PAGE_X))
        .pt(px(20.0))
        .children(back.map(|b| div().flex_none().flex().child(b)))
        .child(head)
        .child(tabs)
        .children(crumbs)
        .child(
            div()
                .id("local-scroll")
                .track_scroll(&app.main_scroll)
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .touch_axis()
                .pb(px(20.0))
                .child(body),
        )
        .into_any_element()
}

/// "Folders › Music › Album" (folders tab), "Artists › Name", "Albums › Title".
fn crumbs(app: &Glyph, roots: &[PathBuf], cx: &mut Context<Glyph>) -> Option<AnyElement> {
    let mut parts: Vec<(String, Option<Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>>)> = Vec::new();
    match app.local.tab {
        LOCAL_FOLDERS => {
            let dir = app.local.dir.clone()?;
            parts.push((t("local.tab.folders").to_string(), Some(Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.open_local_dir(None, cx)))));
            if let Some(root) = roots.iter().find(|r| dir.starts_with(r)) {
                let mut levels = vec![root.clone()];
                if let Ok(rel) = dir.strip_prefix(root) {
                    let mut cur = root.clone();
                    for c in rel.components() {
                        cur = cur.join(c);
                        levels.push(cur.clone());
                    }
                }
                for p in levels {
                    let target = p.clone();
                    parts.push((display_name(&p), Some(Rc::new(move |this: &mut Glyph, cx: &mut Context<Glyph>| this.open_local_dir(Some(target.clone()), cx)))));
                }
            }
        }
        LOCAL_ARTISTS => {
            let name = app.local.artist.clone()?;
            parts.push((t("local.tab.artists").to_string(), Some(Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.open_local_artist(None, cx)))));
            parts.push((name, None));
        }
        LOCAL_ALBUMS => {
            let (title, artist) = app.local.album.clone()?;
            parts.push((t("local.tab.albums").to_string(), Some(Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.open_local_album(None, cx)))));
            parts.push((format!("{title} · {artist}"), None));
        }
        _ => return None,
    }
    let n = parts.len();
    Some(
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(4.0))
            .child(
                icon_button("local-up", "arrow-left", 14.0, theme::text_dim()).on_click(cx.listener(|this, _, _, cx| {
                    this.local_up(cx);
                })),
            )
            .children(parts.into_iter().enumerate().map(|(i, (label, go))| {
                let last = i + 1 == n;
                div()
                    .id(("local-crumb", i))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .when(i > 0, |d| d.child(icon("chevron-right", 13.0, theme::text_faint())))
                    .child(
                        div()
                            .px(px(4.0))
                            .rounded(px(6.0))
                            .text_size(theme::text_sm())
                            .when(last, |d| d.font_weight(FontWeight::SEMIBOLD).text_color(theme::text()))
                            .when(!last, |d| d.text_color(theme::text_dim()).cursor_pointer().hover(|s| s.text_color(theme::text())))
                            .child(label),
                    )
                    .when_some(go.filter(|_| !last), |d, go| d.on_click(cx.listener(move |this, _, _, cx| go(this, cx))))
            }))
            .into_any_element(),
    )
}

fn body(app: &mut Glyph, page: &crate::app::LocalPage, roots: &[PathBuf], cx: &mut Context<Glyph>) -> AnyElement {
    let scroll = app.main_scroll.clone();
    let filtered = !app.local.query.is_empty();
    // No music folders yet: what this is and how to start.
    if roots.is_empty() {
        return div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(14.0))
            .child(ascii_state(ART_EMPTY, t("local.empty.title"), t("local.empty.sub")))
            .child(primary_button("local-add-first", t("local.add")).on_click(cx.listener(|this, _, _, cx| this.add_local_folder(cx))))
            .into_any_element();
    }
    let empty = |title: &str, sub: &str| ascii_state(ART_EMPTY, t(title), t(sub));
    let songs = |app: &mut Glyph, list: Rc<Vec<glyph_protocol::Track>>, cx: &mut Context<Glyph>| -> AnyElement {
        virtual_rows("local-row", list.len(), PITCH, &scroll, |i| song_row(app, list.clone(), i, "local-row", None, true, cx).into_any_element()).into_any_element()
    };
    match app.local.tab {
        LOCAL_FOLDERS if app.local.dir.is_none() => {
            // The music folders themselves; each can be removed again.
            div()
                .flex()
                .flex_col()
                .children(page.dirs.iter().enumerate().map(|(i, (root, n))| {
                    let (open, folder) = (root.clone(), root.to_string_lossy().into_owned());
                    entry_row(("local-root", i), icon_art("folder"), display_name(root), format!("{} · {}", n_songs(*n), folder))
                        .on_click(cx.listener(move |this, _, _, cx| this.open_local_dir(Some(open.clone()), cx)))
                        .child(
                            with_tip(icon_button_plain(("local-remove", i), "x", 14.0, theme::text_dim()), Some(t("local.remove")))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.remove_local_folder(&folder, cx)
                                })),
                        )
                        .into_any_element()
                }))
                .child(div().pt(px(12.0)).text_size(theme::text_xs()).text_color(theme::text_faint()).child(t("local.note")))
                .into_any_element()
        }
        LOCAL_FOLDERS => {
            if page.dirs.is_empty() && page.tracks.is_empty() {
                return if filtered { empty("local.none.title", "local.none.sub") } else { empty("local.folder_empty.title", "local.folder_empty.sub") };
            }
            let dirs = page.dirs.iter().enumerate().map(|(i, (d, n))| {
                let open = d.clone();
                entry_row(("local-dir", i), icon_art("folder"), display_name(d), n_songs(*n))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_local_dir(Some(open.clone()), cx)))
                    .into_any_element()
            });
            let dirs: Vec<AnyElement> = dirs.collect();
            div().flex().flex_col().children(dirs).child(songs(app, page.tracks.clone(), cx)).into_any_element()
        }
        LOCAL_ARTISTS if app.local.artist.is_none() => {
            if page.artists.is_empty() {
                return empty("local.none.title", "local.none.sub").into_any_element();
            }
            let artists = Rc::new(page.artists.clone());
            virtual_rows("local-artist", artists.len(), PITCH, &scroll, |i| {
                let (name, n) = artists[i].clone();
                let open = name.clone();
                entry_row(("local-artist", i), icon_art("user-round"), name, n_songs(n))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_local_artist(Some(open.clone()), cx)))
                    .into_any_element()
            })
            .into_any_element()
        }
        LOCAL_ALBUMS if app.local.album.is_none() => {
            if page.albums.is_empty() {
                return empty("local.none.title", "local.none.sub").into_any_element();
            }
            let albums: Rc<Vec<(String, String, usize, glyph_protocol::Track)>> =
                Rc::new(page.albums.iter().filter_map(|a| Some((a.title.clone(), a.artist.clone(), a.tracks.len(), a.tracks.first()?.clone()))).collect());
            virtual_rows("local-album", albums.len(), PITCH, &scroll, |i| {
                let (title, artist, n, first) = albums[i].clone();
                let art = cover(app, &first, 40.0, 6.0, cx).into_any_element();
                let open = (title.clone(), artist.clone());
                entry_row(("local-album", i), art, title, format!("{artist} · {}", n_songs(n)))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_local_album(Some(open.clone()), cx)))
                    .into_any_element()
            })
            .into_any_element()
        }
        _ => {
            if page.tracks.is_empty() {
                return if filtered || app.local.scanning { empty("local.none.title", "local.none.sub") } else { empty("local.no_songs.title", "local.no_songs.sub") };
            }
            songs(app, page.tracks.clone(), cx)
        }
    }
}
