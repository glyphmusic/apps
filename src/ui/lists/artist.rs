use super::*;

// ------------------------------------------------------------------ artist

pub fn artist_view(app: &mut Glyph, window: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    let Some(name) = app.open_artist.clone() else { return div().into_any_element() };
    let (tracks, loading, info, info_loading, show_all, about_open) = match app.artists.get(&name) {
        Some(p) => (p.tracks.clone(), p.loading, p.info.clone(), p.info_loading, p.show_all, p.about_open),
        None => (Vec::new(), true, None, true, false, false),
    };
    // Artists like this one (Last.fm's artist.getSimilar, else Deezer).
    let similar: Vec<String> = app.artists.get(&name).map(|p| p.similar.clone()).unwrap_or_default();
    // Hero: the artist photo (Wikipedia), else the top song's cover, as ASCII.
    // The cover waits for the info so it never flashes before the photo.
    let src = info
        .as_ref()
        .and_then(|i| i.image.clone())
        .or_else(|| (!info_loading).then(|| tracks.first().cloned().and_then(|tr| app.cover_path(&tr, cx))).flatten());
    // The hero is never wider than the window; a raster at least that wide
    // (for the hero's 420 px height) keeps its bottom fade in view.
    for im in app.stale_heroes.drain(..) {
        let _ = window.drop_image(im);
    }
    let min_w = f32::from(window.viewport_size().width) * 460.0 / 420.0;
    let hero_img = src.as_ref().and_then(|p| app.hero_image(p, min_w, 460, false, cx));
    let still_loading = hero_img.is_none() && (src.is_some() || info_loading || loading);

    let (t_play, t_shuf, t_queue) = (tracks.clone(), tracks.clone(), tracks.clone());
    let has = !tracks.is_empty();
    let n_songs = tracks.len();
    let stats = app.artists.get(&name).and_then(|p| p.stats.clone());
    let verified = stats.as_ref().is_some_and(|s| s.verified);
    let fans = stats.map(|s| s.fans).filter(|f| *f > 0);

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
                        .when(about_open, |d| {
                            let h = crate::ui::widgets::handle_for("artist-about");
                            d.max_h(px(18.0 * 11.0)).overflow_y_scroll().track_scroll(&h).child(crate::ui::widgets::smooth_wheel(&h, false, cx))
                        })
                        .opacity(0.15 + 0.85 * reveal_k(&format!("artist:about:{}", i.url), 0.5))
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
    // Rounded top corners like the panels around it (GPUI clips to rectangles,
    // so the picture rounds its own corners too).
    let hero = div()
        .relative()
        .mx(px(-24.0))
        .mt(px(-12.0))
        .h(px(420.0))
        .rounded_t(px(14.0))
        .overflow_hidden()
        .when_some(hero_img, |d, im| {
            // The picture rises out of an ASCII shimmer.
            let k = reveal_k(&format!("artist:hero:{name}"), 0.8);
            let e = 1.0 - (1.0 - k).powi(2);
            d.child(div().absolute().inset_0().opacity(0.1 + 0.9 * e).child(img(im).size_full().rounded_t(px(14.0)).object_fit(ObjectFit::Cover)))
                .when(k < 1.0, |d| d.child(div().absolute().inset_0().opacity(1.0 - e).child(skeleton("hero-rv", 1680.0, 420.0, 14.0))))
        })
        .when(still_loading, |d| d.child(div().absolute().inset_0().child(skeleton("hero-sk", 1680.0, 420.0, 14.0))))
        .children(back_button(app, cx).map(|b| div().absolute().top(px(20.0)).left(px(32.0)).child(b)))
        .child(
            div()
                .absolute()
                .left(px(32.0))
                .bottom(px(40.0))
                .right(px(if about_card.is_some() { 400.0 } else { 32.0 }))
                .flex()
                .flex_col()
                .child(mono().text_size(px(11.0)).line_height(px(14.0)).text_color(white(0.55)).child(decode(&format!("[{}]", t("artist.badge")), reveal_k(&format!("artist:badge:{name}"), 0.4))))
                .child(
                    div()
                        .pt(px(2.0))
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(46.0))
                                .line_height(px(52.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(gpui::white())
                                .truncate()
                                .child(decode(&name, reveal_k(&format!("artist:name:{name}"), 0.7))),
                        )
                        .when(verified, |d| d.child(verified_badge(24.0))),
                )
                .when(has || fans.is_some(), |d| {
                    let mut parts = Vec::new();
                    if has {
                        parts.push(format!("{:02} {}", n_songs, t("list.songs_caps")));
                    }
                    if let Some(f) = fans {
                        parts.push(tf("artist.fans", &[("n", &compact_count(f))]).to_string());
                    }
                    d.child(mono().pt(px(6.0)).text_size(px(11.5)).text_color(white(0.6)).child(format!("[ {} ]", parts.join(" · "))))
                })
                .when(has, |d| {
                    d.child(
                        div()
                            .pt(px(18.0))
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .child({
                                // One of this artist's songs is on: the button pauses / resumes it.
                                let here = app.current.as_ref().is_some_and(|c| {
                                    let k = glyph_core::dedupe::track_key(c);
                                    c.artist.eq_ignore_ascii_case(&name) || tracks.iter().any(|t| glyph_core::dedupe::track_key(t) == k)
                                });
                                let (playing, loading) = (here && app.is_playing(), here && app.is_loading());
                                round_play("artist-play", playing, loading, 48.0).mr(px(6.0)).on_click(cx.listener(move |this, _, _, cx| {
                                    if here {
                                        this.toggle_play(cx)
                                    } else {
                                        this.play_list(t_play.clone(), 0, cx)
                                    }
                                }))
                            })
                            .child(hero_icon("artist-shuffle", "shuffle").on_click(cx.listener(move |this, _, _, cx| this.play_shuffled(t_shuf.clone(), cx))))
                            .child(follow_button("artist-follow", &name, tracks.first().cloned(), app, 36.0, cx))
                            .child({
                                let nm = name.clone();
                                hero_icon("artist-more", "ellipsis-vertical").on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                                        cx.stop_propagation();
                                        this.open_artist_ctx(&nm, e.position, cx)
                                    }),
                                )
                            }),
                    )
                })
                .when(!has, |d| d.child(div().pt(px(18.0)).flex().child(follow_button("artist-follow", &name, None, app, 36.0, cx)))),
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
                with_list_menu(card().id("artist-liked"), ListRef::ArtistLiked(name.clone()), cx)
                    .flex()
                    .items_center()
                    .gap(px(14.0))
                    .p(px(10.0))
                    .cursor_pointer()
                    .relative()
                    .child(hover_fill("artist-liked", white(0.04), 8.0))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_list(l.clone(), cx)))
                    .child(
                        div()
                            .flex_none()
                            .size(px(56.0))
                            .rounded(px(8.0))
                            .bg(theme::accent_tile(0.22, 0.45))
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

    // Releases, grouped from the songs found; the rest sits on the
    // "Show all" page (one row here, like Spotify's discography).
    // The catalog's discography when known, else the songs found grouped.
    let releases = app.artists.get(&name).map(|p| p.releases.clone()).unwrap_or_default();
    let album_cards: Vec<AnyElement> = if releases.is_empty() {
        artist_releases(&tracks)
            .into_iter()
            .take(ARTIST_ALBUM_ROW)
            .enumerate()
            .map(|(i, (title, list))| album_card(app, i, &name, title, list, 160.0, cx))
            .collect()
    } else {
        releases.into_iter().take(ARTIST_ALBUM_ROW).enumerate().map(|(i, r)| release_card(app, i, r, 160.0, cx)).collect()
    };

    // Similar artists: round cards like the library's artist tiles (picture,
    // name, "Artist"), in a sideways-scrolling row. "Show all" opens the full
    // grid on its own page. Sits at the very bottom of the page, so the popular
    // songs and releases stay in view.
    let similar_section = (!similar.is_empty()).then(|| {
        let cards: Vec<AnyElement> = similar.iter().enumerate().map(|(i, n)| similar_artist_card(app, i, n, cx)).collect();
        let nm = name.clone();
        let all = text_button("artist-similar-all", t("artist.show_all"))
            .text_size(theme::text_sm())
            .font_weight(FontWeight::MEDIUM)
            .on_click(cx.listener(move |this, _, _, cx| this.open_artist_similar(&nm, cx)));
        let h = crate::ui::widgets::handle_for("artist-similar");
        div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(section_title("user-round", t("artist.similar")))
                    .child(div().flex_1())
                    .child(all),
            )
            .child(
                div()
                    .id("artist-similar")
                    .track_scroll(&h)
                    .flex()
                    .gap(px(4.0))
                    .ml(px(-8.0))
                    .mr(px(-32.0))
                    .pr(px(32.0))
                    .overflow_x_scroll()
                    .child(crate::ui::widgets::smooth_wheel(&h, true, cx))
                    .children(cards),
            )
            .into_any_element()
    });

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
            let nm = name.clone();
            d.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(section_title("disc-3", t("artist.albums")))
                            .child(div().flex_1())
                            .child(
                                text_button("artist-albums-all", t("artist.show_all"))
                                    .text_size(theme::text_sm())
                                    .font_weight(FontWeight::MEDIUM)
                                    .on_click(cx.listener(move |this, _, _, cx| this.open_artist_releases(&nm, cx))),
                            ),
                    )
                    .child({
                        let h = crate::ui::widgets::handle_for("artist-albums");
                        div()
                            .id("artist-albums")
                            .track_scroll(&h)
                            .flex()
                            .gap(px(4.0))
                            .ml(px(-8.0))
                            .mr(px(-32.0))
                            .pr(px(32.0))
                            .overflow_x_scroll()
                            .child(crate::ui::widgets::smooth_wheel(&h, true, cx))
                            .children(album_cards)
                    }),
            )
        })
        .children(similar_section);
    div().id("artist-scroll").track_scroll(&app.main_scroll).size_full().overflow_y_scroll().px(px(32.0)).py(px(20.0)).child(body).into_any_element()
}

/// Releases in the artist page's row; the rest sits behind "Show all".
pub(super) const ARTIST_ALBUM_ROW: usize = 6;

/// An artist's releases: the songs found, grouped by album name.
pub(super) fn artist_releases(tracks: &[Track]) -> Vec<(String, Vec<Track>)> {
    let mut albums: Vec<(String, Vec<Track>)> = Vec::new();
    for tr in tracks {
        if let Some(a) = tr.album.clone().filter(|a| !a.trim().is_empty()) {
            match albums.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(&a)) {
                Some((_, v)) => v.push(tr.clone()),
                None => albums.push((a, vec![tr.clone()])),
            }
        }
    }
    albums
}

/// One round artist card (picture, name, "Artist") for the similar-artist row
/// and its "Show all" grid.
pub(super) fn similar_artist_card(app: &mut Glyph, i: usize, n: &str, cx: &mut Context<Glyph>) -> AnyElement {
    let nm = n.to_string();
    let photo = app.artists.get(n).and_then(|p| p.info.as_ref()).and_then(|info| info.image.clone());
    let art: AnyElement = match photo {
        Some(path) => img(path).size(px(168.0)).rounded_full().object_fit(ObjectFit::Cover).into_any_element(),
        None => match similar_artist_track(app, n) {
            Some(tr) => cover(app, &tr, 168.0, 84.0, cx).into_any_element(),
            None => div()
                .flex_none()
                .size(px(168.0))
                .rounded_full()
                .bg(theme::card())
                .flex()
                .items_center()
                .justify_center()
                .child(icon("user-round", 48.0, white(0.5)))
                .into_any_element(),
        },
    };
    with_artist_menu(div().id(("artist-sim", i)), n.to_string(), cx)
        .flex_none()
        .w(px(184.0))
        .p(px(8.0))
        .rounded(px(10.0))
        .flex()
        .flex_col()
        .gap(px(4.0))
        .cursor_pointer()
        .relative()
        .child(hover_fill(format!("artist-sim-{i}"), white(0.05), 10.0))
        .on_click(cx.listener(move |this, _, _, cx| this.open_artist(&nm, cx)))
        .child(art)
        .child(artist_name_badged(app, n, n.to_string(), 14.0, cx).pt(px(6.0)).text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).text_color(theme::text()))
        .child(div().text_size(theme::text_sm()).text_color(theme::text_dim()).child(t("search.kind.artist")))
        .into_any_element()
}

/// A song already known for `name`, used as the picture on its similar-artist
/// tile (their open page, a followed artist or recent plays). `None` falls
/// back to the round placeholder.
pub(super) fn similar_artist_track(app: &Glyph, name: &str) -> Option<Track> {
    if let Some(tr) = app.artists.get(name).and_then(|p| p.tracks.first()).cloned() {
        return Some(tr);
    }
    if let Some(tr) = app.library.followed.iter().find(|f| f.name.eq_ignore_ascii_case(name)).and_then(|f| f.track.clone()) {
        return Some(tr);
    }
    if let Some(tr) = app
        .library
        .recent
        .iter()
        .filter_map(|r| r.track())
        .find(|t| glyph_core::dedupe::by_artist(&t.artist, name))
        .cloned()
    {
        return Some(tr);
    }
    None
}

/// One catalog release (cover, title, "year · kind"); opens it and loads its songs.
pub(super) fn release_card(app: &mut Glyph, i: usize, r: glyph_core::providers::deezer::Release, size: f32, cx: &mut Context<Glyph>) -> AnyElement {
    let seed = r.cover_track();
    let art = cover(app, &seed, size, 8.0, cx);
    let kind = match r.kind {
        glyph_core::catalog::AlbumKind::Album => t("album.kind.album"),
        glyph_core::catalog::AlbumKind::Ep => t("album.kind.ep"),
        glyph_core::catalog::AlbumKind::Single => t("album.kind.single"),
        glyph_core::catalog::AlbumKind::Compilation => t("album.kind.compilation"),
    };
    let label = match r.year {
        Some(y) => format!("{y} · {kind}"),
        None => kind.to_string(),
    };
    let title = r.title.clone();
    let lm = ListRef::Album { title: r.title.clone(), artist: r.artist.clone(), tracks: vec![seed] };
    with_list_menu(div().id(("release", i)), lm, cx)
        .flex_none()
        .w(px(size + 16.0))
        .p(px(8.0))
        .rounded(px(10.0))
        .relative()
        .child(hover_fill(format!("release-{i}"), white(0.05), 10.0))
        .flex()
        .flex_col()
        .gap(px(4.0))
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| this.open_release(r.clone(), cx)))
        .child(art)
        .child(div().pt(px(4.0)).truncate().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).text_color(theme::text()).child(title))
        .child(mono().text_size(px(11.0)).text_color(theme::text_faint()).child(label.to_uppercase()))
        .into_any_element()
}

/// One release card (cover, title, "year · kind"); opens the album's list.
pub(super) fn album_card(app: &mut Glyph, i: usize, artist: &str, title: String, list: Vec<Track>, size: f32, cx: &mut Context<Glyph>) -> AnyElement {
    let art = cover(app, &list[0], size, 8.0, cx);
    let label = app.album_label(&title, artist, list.len(), cx);
    let l = ListRef::Album { title: title.clone(), artist: artist.to_string(), tracks: list };
    let lm = l.clone();
    with_list_menu(div().id(("album", i)), lm, cx)
        .flex_none()
        .w(px(size + 16.0))
        .p(px(8.0))
        .rounded(px(10.0))
        .relative()
        .child(hover_fill(format!("album-{i}"), white(0.05), 10.0))
        .flex()
        .flex_col()
        .gap(px(4.0))
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| this.open_list(l.clone(), cx)))
        .child(art)
        .child(div().pt(px(4.0)).truncate().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).text_color(theme::text()).child(title))
        .child(mono().text_size(px(11.0)).text_color(theme::text_faint()).child(label.to_uppercase()))
        .into_any_element()
}

/// "Show all" of an artist's releases: every album (and single) on one page.
pub fn releases_view(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    let Some(name) = app.open_artist.clone() else { return div().into_any_element() };
    let tracks = app.artists.get(&name).map(|p| p.tracks.clone()).unwrap_or_default();
    let loading = app.artists.get(&name).is_none_or(|p| p.loading) && tracks.is_empty();
    let releases = app.artists.get(&name).map(|p| p.releases.clone()).unwrap_or_default();
    let back = back_button(app, cx);
    let cards: Vec<AnyElement> = if releases.is_empty() {
        artist_releases(&tracks)
            .into_iter()
            .enumerate()
            .map(|(i, (title, list))| album_card(app, i, &name, title, list, 168.0, cx))
            .collect()
    } else {
        releases.into_iter().enumerate().map(|(i, r)| release_card(app, i, r, 168.0, cx)).collect()
    };
    let body: AnyElement = if cards.is_empty() {
        if loading {
            skeleton_artist_rows("releases", 2).into_any_element()
        } else {
            ascii_state(ART_EMPTY, t("artist.empty"), t("artist.empty.sub")).into_any_element()
        }
    } else {
        div().flex().flex_wrap().gap(px(4.0)).ml(px(-8.0)).children(cards).into_any_element()
    };
    div()
        .id("releases-scroll")
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
                .children(back)
                .child(heading(format!("{} · {}", name, t("artist.albums"))))
                .child(body),
        )
        .into_any_element()
}

/// "Show all" of an artist's similar artists: every card in one grid.
pub fn similar_view(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    let Some(name) = app.open_artist.clone() else { return div().into_any_element() };
    let similar = app.artists.get(&name).map(|p| p.similar.clone()).unwrap_or_default();
    let loading = app.artists.get(&name).is_none_or(|p| p.similar_loading);
    let back = back_button(app, cx);
    let cards: Vec<AnyElement> = similar.iter().enumerate().map(|(i, n)| similar_artist_card(app, i, n, cx)).collect();
    let body: AnyElement = if cards.is_empty() {
        if loading {
            skeleton_artist_rows("similar", 2).into_any_element()
        } else {
            ascii_state(ART_EMPTY, t("artist.similar"), String::new()).into_any_element()
        }
    } else {
        div().flex().flex_wrap().gap(px(4.0)).ml(px(-8.0)).children(cards).into_any_element()
    };
    div()
        .id("similar-scroll")
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
                .children(back)
                .child(heading(format!("{} · {}", name, t("artist.similar"))))
                .child(body),
        )
        .into_any_element()
}
