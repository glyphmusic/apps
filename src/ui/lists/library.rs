use super::*;

// ------------------------------------------------------------------ library

pub fn library(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    let stats_btn = div()
        .id("lib-stats")
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
        .on_click(cx.listener(|this, _, _, cx| this.open_stats(cx)))
        .child(icon("chart-column", 14.0, white(0.9)))
        .child(t("stats.title"));
    let migrate_btn = div()
        .id("lib-migrate")
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
        .on_click(cx.listener(|this, _, _, cx| this.open_migrate(cx)))
        .child(icon("arrow-right-left", 14.0, white(0.9)))
        .child(t("library.migrate"));
    let banner = crate::ui::migrate::library_banner(app, cx);
    let head = div()
        .flex()
        .items_center()
        .justify_between()
        .child(heading(t("nav.library")))
        .child(div().flex().gap(px(8.0)).child(migrate_btn).child(stats_btn).child(
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
                .on_click(cx.listener(|this, _, w, cx| this.open_name_dialog(Vec::new(), None, w, cx)))
                .child(icon("plus", 14.0, white(0.9)))
                .child(t("library.playlist.new")),
        ));
    if !app.library_ok {
        return div()
            .id("lib-scroll")
        .track_scroll(&app.main_scroll)
            .size_full()
            .px(px(theme::PAGE_X))
            .py(px(20.0))
            .child(div().flex().flex_col().gap(px(18.0)).child(head).child(ascii_state(ART_OFFLINE, t("status.library_offline"), t("library.offline.sub"))))
            .into_any_element();
    }
    let mut lists: Vec<(ListRef, String)> = vec![(ListRef::Liked, t("list.kind.playlist").to_string())];
    if !app.library.recent.is_empty() {
        lists.push((ListRef::Recent, t("list.kind.playlist").to_string()));
    }
    if app.backend.as_ref().is_some_and(|b| !b.downloaded_tracks().is_empty()) {
        lists.push((ListRef::Downloads, t("list.kind.playlist").to_string()));
    }
    for p in &app.library.playlists {
        lists.push((ListRef::User(p.id.clone()), t(if p.local_only || app.signed_in().is_none() { "playlist.scope.local" } else if p.auto_download { "playlist.scope.offline" } else { "playlist.scope.account" }).to_string()));
    }
    for k in &app.library.saved_albums {
        let l = ListRef::from_key(k);
        let by = match &l {
            ListRef::Album { artist, .. } => format!("{} · {artist}", t("list.kind.album")),
            _ => t("list.kind.album").to_string(),
        };
        lists.push((l, by));
    }
    // Pinned lists first, in the order they were pinned.
    let pin_rank = |l: &ListRef| app.library.pinned.iter().position(|p| *p == l.key()).unwrap_or(usize::MAX);
    lists.sort_by_key(|(l, _)| pin_rank(l));
    let tiles: Vec<AnyElement> = lists
        .into_iter()
        .enumerate()
        .map(|(i, (l, kind))| {
            let pinned = app.is_pinned(&l);
            let art = list_cover(app, &l, 168.0, cx);
            // A small pin on the cover of pinned lists.
            let art = div()
                .relative()
                .child(art)
                .when(pinned, |d| {
                    d.child(
                        div()
                            .absolute()
                            .top(px(8.0))
                            .right(px(8.0))
                            .size(px(24.0))
                            .rounded_full()
                            .bg(gpui::hsla(0.0, 0.0, 0.0, 0.6))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon("pin-fill", 12.0, theme::pink())),
                    )
                })
                .into_any_element();
            let title = app.list_title(&l);
            let n = app.list_tracks(&l).len();
            let lm = l.clone();
            with_list_menu(div().id(("lib-tile", i)), lm, cx)
                .flex_none()
                .w(px(184.0))
                .p(px(8.0))
                .rounded(px(10.0))
                .flex()
                .flex_col()
                .gap(px(4.0))
                .cursor_pointer()
                .relative()
                .child(hover_fill(format!("lib-tile-{i}"), white(0.05), 10.0))
                .on_click(cx.listener(move |this, _, _, cx| this.open_list(l.clone(), cx)))
                .child(art)
                .child(div().pt(px(6.0)).truncate().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).text_color(theme::text()).child(title))
                .child(div().truncate().text_size(theme::text_sm()).text_color(theme::text_dim()).child(format!("{kind} · {}", tf("common.n_songs", &[("n", &n)]))))
                .into_any_element()
        })
        .collect();
    // Followed artists: round pictures, open the artist page.
    let followed = app.library.followed.clone();
    let artist_tiles: Vec<AnyElement> = followed
        .into_iter()
        .enumerate()
        .map(|(i, f)| {
            let art: AnyElement = match &f.track {
                Some(tr) => cover(app, tr, 168.0, 84.0, cx).into_any_element(),
                None => div()
                    .size(px(168.0))
                    .rounded_full()
                    .bg(theme::card())
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("user-round", 48.0, white(0.5)))
                    .into_any_element(),
            };
            let name = f.name.clone();
            with_artist_menu(div().id(("lib-artist", i)), f.name.clone(), cx)
                .flex_none()
                .w(px(184.0))
                .p(px(8.0))
                .rounded(px(10.0))
                .flex()
                .flex_col()
                .gap(px(4.0))
                .cursor_pointer()
                .relative()
                .child(hover_fill(format!("lib-artist-{i}"), white(0.05), 10.0))
                .on_click(cx.listener(move |this, _, _, cx| this.open_artist(&name, cx)))
                .child(art)
                .child(artist_name_badged(app, &f.name, f.name.clone(), 14.0, cx).pt(px(6.0)).text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).text_color(theme::text()))
                .child(div().text_size(theme::text_sm()).text_color(theme::text_dim()).child(t("search.kind.artist")))
                .into_any_element()
        })
        .collect();
    let artists_section = (!artist_tiles.is_empty()).then(|| {
        div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .pt(px(10.0))
            .child(section_title("user-round", t("library.artists")))
            .child(div().flex().flex_wrap().gap(px(8.0)).ml(px(-8.0)).children(artist_tiles))
    });
    let trash: Vec<AnyElement> = app.library.playlist_trash.iter().enumerate().map(|(n, (id, p))| {
        let id = id.clone();
        div().id(("playlist-trash", n)).flex().items_center().gap(px(10.0)).cursor_pointer()
            .child(icon("history", 16.0, white(0.7)))
            .child(tf("playlist.restore", &[("name", &p.name)]))
            .on_click(cx.listener(move |s, _, _, cx| s.restore_playlist(&id, cx))).into_any_element()
    }).collect();
    div()
        .id("lib-scroll")
        .track_scroll(&app.main_scroll)
        .size_full()
        .overflow_y_scroll()
        .px(px(theme::PAGE_X))
        .py(px(20.0))
        .child(div().flex().flex_col().gap(px(18.0)).child(head).children(banner).child(div().flex().flex_wrap().gap(px(8.0)).ml(px(-8.0)).children(tiles)).children(artists_section).children(trash))
        .into_any_element()
}
