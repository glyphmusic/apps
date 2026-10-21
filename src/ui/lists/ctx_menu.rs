use super::*;

// ------------------------------------------------------------------ song menu

/// Width of the song menu and of its playlist flyout.
pub(super) const MENU_W: f32 = 244.0;
pub(super) const SUB_W: f32 = 228.0;
/// Space between the menu and its flyout.
pub(super) const SUB_GAP: f32 = 5.0;
/// A menu row's left edge inside the menu (1 px border + 6 px padding).
pub(super) const MENU_INSET: f32 = 7.0;

thread_local! {
    /// The open menu's bounds in the window (measured while painting): the
    /// flyout opens on the side where it fits.
    pub(super) static MENU_BOUNDS: std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>> = const { std::cell::Cell::new(None) };
    /// Phones: the open submenu (title, rows), shown as its own sheet.
    static SHEET_SUB: std::cell::RefCell<Option<(SharedString, Vec<AnyElement>)>> = const { std::cell::RefCell::new(None) };
}

/// Flyouts of the menu (`CtxMenu::sub`).
pub(super) const SUB_PLAYLIST: u8 = 0;
pub(super) const SUB_RECOMMEND: u8 = 1;
pub(super) const SUB_STORAGE: u8 = 2;
pub(super) const SUB_VERSIONS: u8 = 3;

/// "Add to playlist" row with its hover flyout (`sub`).
pub(super) fn add_pl_row(open: bool, flip: bool, sub: Vec<AnyElement>, whole_list: bool, cx: &mut Context<Glyph>) -> AnyElement {
    // On a list it copies all of its songs, so it says so.
    let label = if whole_list { t("ctx.add_all_playlist") } else { t("ctx.add_playlist") };
    sub_row(SUB_PLAYLIST, "list-music", label, open, flip, sub, cx)
}

/// Row with a hover flyout (`sub`) of kind `kind`. The flyout keeps
/// `SUB_GAP` from the menu and stays inside the window (it slides up near the
/// bottom edge and opens to the left near the right edge).
pub(super) fn sub_row(kind: u8, name: &str, label: SharedString, open: bool, flip: bool, sub: Vec<AnyElement>, cx: &mut Context<Glyph>) -> AnyElement {
    let id: ElementId = ("ctx-sub-row", kind as usize).into();
    // Phones (bottom sheet): no hover; a tap swaps the sheet for one with
    // just the submenu's rows (see `sheet`).
    if crate::ui::compact::TOUCH {
        if open {
            SHEET_SUB.with(|s| *s.borrow_mut() = Some((label.clone(), sub)));
        }
        return ctx_item(id, name, label, white(0.7), false, cx)
            .child(icon("chevron-right", 16.0, white(0.4)))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(m) = this.ctx_menu.as_mut() {
                    m.playlists = true;
                    m.sub = kind;
                    // Slides in like a new sheet.
                    this.ctx_opened = std::time::Instant::now();
                    crate::ui::widgets::handle_for("ctx-sheet").set_offset(gpui::point(px(0.0), px(0.0)));
                    cx.notify();
                }
            }))
            .into_any_element();
    }
    ctx_item(id, name, label, white(0.7), false, cx)
                .relative()
                .when(open, |d| d.bg(white(0.07)))
                .child(icon("chevron-right", 14.0, white(0.4)))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if let Some(m) = this.ctx_menu.as_mut().filter(|m| *hovered && !(m.playlists && m.sub == kind)) {
                        m.playlists = true;
                        m.sub = kind;
                        cx.notify();
                    }
                }))
                .when(open, |d| {
                    let flyout = div()
                        .id("ctx-sub")
                        .track_scroll(&crate::ui::widgets::handle_for("ctx-sub"))
                        .child(crate::ui::widgets::smooth_wheel(&crate::ui::widgets::handle_for("ctx-sub"), false, cx))
                        .occlude()
                        .w(px(SUB_W))
                        .max_h(px(320.0))
                        .overflow_y_scroll().touch_axis()
                        .p(px(6.0))
                        .rounded(px(10.0))
                        .bg(gpui::hsla(0.0, 0.0, 0.075, 0.98))
                        .border_1()
                        .border_color(white(0.1))
                        .flex()
                        .flex_col()
                        .font_family(theme::FONT)
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .children(sub);
                    // The corner the flyout hangs from, just outside the menu;
                    // `anchored` moves it back inside the window when needed.
                    let corner = div()
                        .absolute()
                        .top(px(-6.0))
                        .left(px(if flip { -(MENU_INSET + SUB_GAP) } else { MENU_W - MENU_INSET + SUB_GAP }));
                    d.child(corner.child(
                        anchored()
                            .anchor(if flip { gpui::Anchor::TopRight } else { gpui::Anchor::TopLeft })
                            .snap_to_window_with_margin(px(8.0))
                            .child(flyout),
                    ))
                })
                .into_any_element()
}

/// Destructive row (remove, hide, delete): turns red on hover.
pub(super) fn danger_item(id: ElementId, name: &str, label: SharedString, cx: &mut Context<Glyph>) -> Stateful<gpui::Div> {
    menu_row(id, name, label, gpui::hsla(0.0, 0.75, 0.68, 0.9), true, true, cx)
}

/// One menu row. `closes_sub`: hovering it closes the playlist flyout.
pub(super) fn ctx_item(id: ElementId, name: &str, label: SharedString, color: Hsla, closes_sub: bool, cx: &mut Context<Glyph>) -> Stateful<gpui::Div> {
    menu_row(id, name, label, color, closes_sub, false, cx)
}

/// A div takes only one hover style, so normal and red rows share this.
pub(crate) fn menu_row(id: ElementId, name: &str, label: SharedString, color: Hsla, closes_sub: bool, danger: bool, cx: &mut Context<Glyph>) -> Stateful<gpui::Div> {
    // Hover fades in and out; red rows also fade their text to red.
    let key = format!("menu-{id:?}");
    let k = hover_k(&key);
    let text = if danger { mix(white(0.88), gpui::hsla(0.0, 0.85, 0.75, 1.0), k) } else { white(0.88) };
    let touch = crate::ui::compact::TOUCH;
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(if touch { 16.0 } else { 10.0 }))
        .h(px(if touch { 50.0 } else { 32.0 }))
        .px(px(10.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .text_size(px(if touch { 15.0 } else { 13.0 }))
        .text_color(text)
        .relative()
        .child(hover_fill(key, if danger { gpui::hsla(0.0, 0.7, 0.5, 0.18) } else { white(0.07) }, 6.0))
        .when(closes_sub && !touch, |d| {
            d.on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if let Some(m) = this.ctx_menu.as_mut().filter(|m| *hovered && m.playlists) {
                    m.playlists = false;
                    cx.notify();
                }
            }))
        })
        .child(icon(name, if touch { 20.0 } else { 15.0 }, color))
        .child(div().flex_1().truncate().child(label))
}

pub fn ctx_menu(app: &mut Glyph, window: &mut Window, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    // Open: fade + slide in (140 ms). Closing: fade out (110 ms), not clickable.
    const IN: f32 = 0.14;
    const OUT: f32 = 0.11;
    if app.ctx_closing.as_ref().is_some_and(|(_, at)| at.elapsed().as_secs_f32() >= OUT) {
        app.ctx_closing = None;
    }
    let (m, alpha, live) = match (&app.ctx_menu, &app.ctx_closing) {
        (Some(m), _) => {
            let k = (app.ctx_opened.elapsed().as_secs_f32() / IN).min(1.0);
            (m.clone(), 1.0 - (1.0 - k).powi(3), true)
        }
        (None, Some((m, at))) => (m.clone(), 1.0 - (at.elapsed().as_secs_f32() / OUT).min(1.0), false),
        _ => return None,
    };
    SHEET_SUB.with(|s| s.borrow_mut().take());
    let touch = crate::ui::compact::TOUCH;
    let tr = m.track.clone();
    let sep = || div().h(px(1.0)).mx(px(6.0)).my(px(4.0)).bg(white(0.07));
    let mut body: Vec<AnyElement> = Vec::new();
    // Songs the menu acts on: one song, or a whole list.
    let songs: Vec<Track> = match &m.list {
        Some(l) => app.shown_tracks(l),
        None => vec![tr.clone()],
    };
    // "Add to playlist" flyout (opens on hover, to the right or left).
    let is_open = |k: u8| m.playlists && m.sub == k;
    let mut sub: Vec<AnyElement> = Vec::new();
    if is_open(SUB_PLAYLIST) {
        let t_new = songs.clone();
        sub.push(
            ctx_item("ctx-new-pl".into(), "plus", t("library.playlist.new"), white(0.7), false, cx)
                .on_click(cx.listener(move |this, _, w, cx| this.open_name_dialog(t_new.clone(), None, w, cx)))
                .into_any_element(),
        );
        let own = match &m.list {
            Some(ListRef::User(id)) => Some(id.clone()),
            _ => None,
        };
        let pls: Vec<(String, String)> = app
            .library
            .playlists
            .iter()
            .filter(|p| Some(&p.id) != own.as_ref())
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect();
        sub.push(sep().into_any_element());
        if pls.is_empty() {
            // Nothing to add to yet: say so instead of an empty list.
            sub.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .px(px(10.0))
                    .py(px(8.0))
                    .child(div().text_size(px(13.0)).text_color(white(0.75)).child(t("ctx.no_playlists")))
                    .child(div().text_size(px(12.0)).text_color(white(0.45)).child(t("ctx.no_playlists.sub")))
                    .into_any_element(),
            );
        }
        for (i, (id, name)) in pls.into_iter().enumerate() {
            let t_add = songs.clone();
            sub.push(
                ctx_item(("ctx-pl", i).into(), "list-music", name.into(), white(0.7), false, cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.begin_ctx_close();
                        this.add_many_to_playlist(&id, t_add.clone(), cx)
                    }))
                    .into_any_element(),
            );
        }
    }
    // Flyout on the left when it would not fit right of the menu.
    let menu_right = MENU_BOUNDS.with(|b| b.get()).map_or(f32::from(m.pos.x) + MENU_W, |b| f32::from(b.right()));
    let flip = menu_right + SUB_GAP + SUB_W + 8.0 > f32::from(window.viewport_size().width);
    if let Some(name) = m.artist.clone() {
        body.push(
            div()
                .px(px(8.0))
                .py(px(6.0))
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(white(0.95))
                .truncate()
                .child(name.clone())
                .into_any_element(),
        );
        body.push(sep().into_any_element());
        let songs = app.artists.get(&name).map(|p| p.tracks.clone()).unwrap_or_default();
        if !songs.is_empty() {
            let (s1, s2) = (songs.clone(), songs.clone());
            body.push(
                ctx_item("actx-queue".into(), "list-plus", t("ctx.queue"), white(0.7), true, cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.begin_ctx_close();
                        this.queue_all(s1.clone(), cx)
                    }))
                    .into_any_element(),
            );
            body.push(
                ctx_item("actx-next".into(), "list-start", t("ctx.play_next"), white(0.7), true, cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.begin_ctx_close();
                        for tr in s2.iter().rev() {
                            this.queue_add(tr.clone(), true, cx);
                        }
                    }))
                    .into_any_element(),
            );
        }
        let n_radio = name.clone();
        body.push(
            ctx_item("actx-radio".into(), "radio", t("ctx.artist_radio"), white(0.7), true, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.begin_ctx_close();
                    this.artist_radio(&n_radio, cx)
                }))
                .into_any_element(),
        );
        let following = app.is_following(&name);
        let (n1, n2, n3) = (name.clone(), name.clone(), name.clone());
        let first = songs.first().cloned();
        body.push(
            ctx_item(
                "actx-follow".into(),
                if following { "check" } else { "user-round" },
                if following { t("artist.unfollow") } else { t("artist.follow") },
                white(0.7),
                true,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.begin_ctx_close();
                this.toggle_follow(&n1, first.clone(), cx)
            }))
            .into_any_element(),
        );
        body.push(
            ctx_item("actx-link".into(), "link-2", t("ctx.copy_link"), white(0.7), true, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.begin_ctx_close();
                    this.copy_artist_link(&n2, cx);
                    cx.notify();
                }))
                .into_any_element(),
        );
        body.push(sep().into_any_element());
        let hidden = app.library.is_hidden(&name);
        body.push(
            if hidden {
                ctx_item("actx-hide".into(), "eye", t("ctx.show_artist"), white(0.7), true, cx)
            } else {
                danger_item("actx-hide".into(), "eye-off", t("ctx.hide_artist"), cx)
            }
            .on_click(cx.listener(move |this, _, _, cx| {
                this.begin_ctx_close();
                this.toggle_hidden_artist(&n3, cx)
            }))
            .into_any_element(),
        );
    } else if let Some(l) = m.list.clone() {
        let art = list_cover(app, &l, 36.0, cx);
        let title = app.list_title(&l);
        let kind = match &l {
            ListRef::Album { artist, .. } => artist.clone(),
            _ => t("list.kind.playlist").to_string(),
        };
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
                        .child(div().truncate().text_size(px(13.0)).font_weight(FontWeight::MEDIUM).text_color(white(0.95)).child(title))
                        .child(div().truncate().text_size(px(12.0)).text_color(white(0.55)).child(kind)),
                )
                .into_any_element(),
        );
        body.push(sep().into_any_element());
        let has = !songs.is_empty();
        // The most used actions first: save and add to playlist.
        let album = matches!(l, ListRef::Album { .. });
        if album {
            let saved = app.is_saved(&l);
            let l_s = l.clone();
            body.push(
                ctx_item(
                    "lctx-save".into(),
                    if saved { "heart-fill" } else { "heart" },
                    if saved { t("ctx.unsave") } else { t("ctx.save") },
                    if saved { theme::pink() } else { white(0.7) },
                    true,
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.begin_ctx_close();
                    this.toggle_saved(&l_s, cx)
                }))
                .into_any_element(),
            );
        }
        if has {
            body.push(add_pl_row(is_open(SUB_PLAYLIST), flip, sub, true, cx));
        }
        if album || has {
            body.push(sep().into_any_element());
        }
        if has {
            let (s1, s2, s3) = (songs.clone(), songs.clone(), songs.clone());
            body.push(
                ctx_item("lctx-play".into(), "play", t("ctx.play"), white(0.7), true, cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.begin_ctx_close();
                        this.play_list(s1.clone(), 0, cx)
                    }))
                    .into_any_element(),
            );
            body.push(
                ctx_item("lctx-shuffle".into(), "shuffle", t("ctx.shuffle"), white(0.7), true, cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.begin_ctx_close();
                        this.play_shuffled(s2.clone(), cx)
                    }))
                    .into_any_element(),
            );
            body.push(
                ctx_item("lctx-queue".into(), "list-plus", t("ctx.queue"), white(0.7), true, cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.begin_ctx_close();
                        this.queue_all(s3.clone(), cx)
                    }))
                    .into_any_element(),
            );
            let l_radio = l.clone();
            body.push(
                ctx_item("lctx-radio".into(), "radio", t("ctx.radio"), white(0.7), true, cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.begin_ctx_close();
                        this.list_radio(&l_radio, cx)
                    }))
                    .into_any_element(),
            );
            let s4 = songs.clone();
            if songs.iter().all(|tr| app.is_downloaded(tr)) {
                body.push(
                    danger_item("lctx-undl".into(), "trash-2", t("ctx.remove_downloads"), cx)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.begin_ctx_close();
                            this.remove_downloads(s4.clone(), cx)
                        }))
                        .into_any_element(),
                );
            } else {
                body.push(
                    ctx_item("lctx-dl".into(), "download", t("ctx.download_all"), white(0.7), true, cx)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.begin_ctx_close();
                            this.download_tracks(s4.clone(), cx)
                        }))
                        .into_any_element(),
                );
            }
            body.push(sep().into_any_element());
        }
        let (l_pin, pinned) = (l.clone(), app.is_pinned(&l));
        body.push(
            ctx_item("lctx-pin".into(), if pinned { "pin-fill" } else { "pin" }, if pinned { t("ctx.unpin") } else { t("ctx.pin") }, white(0.7), true, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.begin_ctx_close();
                    this.toggle_pin(&l_pin, cx)
                }))
                .into_any_element(),
        );
        // Phones open the list with a tap on it.
        if !touch {
            let l_open = l.clone();
            body.push(
                ctx_item("lctx-open".into(), "list-music", t("ctx.open"), white(0.7), true, cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.begin_ctx_close();
                        this.open_list(l_open.clone(), cx)
                    }))
                    .into_any_element(),
            );
        }
        if has {
            let l_share = l.clone();
            body.push(
                ctx_item("lctx-share".into(), "link-2", t("ctx.share_list"), white(0.7), true, cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.begin_ctx_close();
                        this.share_list(&l_share, cx)
                    }))
                    .into_any_element(),
            );
        }
        match &l {
            ListRef::Album { artist, .. } | ListRef::ArtistLiked(artist) => {
                let a = artist.clone();
                body.push(
                    ctx_item("lctx-artist".into(), "user-round", t("ctx.artist"), white(0.7), true, cx)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.begin_ctx_close();
                            this.open_artist(&a, cx)
                        }))
                        .into_any_element(),
                );
            }
            ListRef::User(id) => {
                body.push(sep().into_any_element());
                let (id1, id2) = (id.clone(), id.clone());
                body.push(
                    ctx_item("lctx-rename".into(), "pencil", t("list.edit"), white(0.7), true, cx)
                        .on_click(cx.listener(move |this, _, w, cx| this.open_name_dialog(Vec::new(), Some(id1.clone()), w, cx)))
                        .into_any_element(),
                );
                // Storage and older versions live in flyouts.
                if app.signed_in().is_some() {
                    let mut rows = Vec::new();
                    for (mode, key) in [(0u8, "playlist.scope.local"), (1, "playlist.scope.account"), (2, "playlist.scope.offline")] {
                        let scope_id = id.clone();
                        rows.push(ctx_item(format!("lctx-storage-{mode}").into(), "hard-drive", t(key), white(0.7), false, cx)
                            .on_click(cx.listener(move |s, _, _, cx| { s.begin_ctx_close(); s.set_playlist_storage(&scope_id, mode, cx); }))
                            .into_any_element());
                    }
                    body.push(sub_row(SUB_STORAGE, "hard-drive", t("ctx.storage"), is_open(SUB_STORAGE), flip, rows, cx));
                }
                let mut versions: Vec<_> = app.library.playlist_versions.iter().filter(|(_, v)| &v.playlist.id == id && app.library.playlist(id) != Some(&v.playlist)).map(|(k, v)| (k.clone(), v.saved_at)).collect();
                versions.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
                if !versions.is_empty() {
                    let mut rows = Vec::new();
                    for (n, (revision, at)) in versions.into_iter().take(10).enumerate() {
                        rows.push(ctx_item(format!("lctx-version-{n}").into(), "history", tf("playlist.restore_version", &[("date", &at)]), white(0.7), false, cx)
                            .on_click(cx.listener(move |s, _, _, cx| { s.begin_ctx_close(); s.restore_playlist_version(&revision, cx); })).into_any_element());
                    }
                    body.push(sub_row(SUB_VERSIONS, "history", t("ctx.versions"), is_open(SUB_VERSIONS), flip, rows, cx));
                }
                body.push(
                    danger_item("lctx-delete".into(), "trash-2", t("list.delete"), cx)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.begin_ctx_close();
                            this.delete_playlist(&id2, cx)
                        }))
                        .into_any_element(),
                );
            }
            _ => {}
        }
    } else {
        // Phones: the sheet has its own, bigger song header.
        if !touch {
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
        }
        // The most used actions first: like and add to playlist.
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
                this.begin_ctx_close();
                this.toggle_like(&t4.clone(), cx)
            }))
            .into_any_element(),
        );
        body.push(add_pl_row(is_open(SUB_PLAYLIST), flip, sub, false, cx));
        body.push(sep().into_any_element());
        // Phones play a song with a tap on its row.
        if !touch {
            let t1 = tr.clone();
            body.push(
                ctx_item("ctx-play".into(), "play", t("ctx.play"), white(0.7), true, cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.begin_ctx_close();
                        this.play_list(vec![t1.clone()], 0, cx)
                    }))
                    .into_any_element(),
            );
        }
        let t2 = tr.clone();
        body.push(
            ctx_item("ctx-next".into(), "list-start", t("ctx.play_next"), white(0.7), true, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.begin_ctx_close();
                    this.queue_add(t2.clone(), true, cx)
                }))
                .into_any_element(),
        );
        let t3 = tr.clone();
        body.push(
            ctx_item("ctx-queue".into(), "list-plus", t("ctx.queue"), white(0.7), true, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.begin_ctx_close();
                    this.queue_add(t3.clone(), false, cx)
                }))
                .into_any_element(),
        );
        let t_radio = tr.clone();
        body.push(
            ctx_item("ctx-radio".into(), "radio", t("ctx.radio"), white(0.7), true, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.begin_ctx_close();
                    let title = t_radio.title.clone();
                    this.start_radio_from(vec![t_radio.clone()], title, cx)
                }))
                .into_any_element(),
        );
        // Recommendation feedback for the artist, in a flyout.
        let mut rows = Vec::new();
        for (key, preference) in [("recommend.more", Some(1i8)), ("recommend.less", Some(-1)), ("recommend.block", Some(-2)), ("recommend.reset", None)] {
            let artist = tr.artist.clone();
            rows.push(ctx_item(key.into(), "sliders-horizontal", t(key), white(0.7), false, cx)
                .on_click(cx.listener(move |s, _, _, cx| { s.begin_ctx_close(); s.recommendation_feedback(&artist, preference, cx); })).into_any_element());
        }
        body.push(sub_row(SUB_RECOMMEND, "sliders-horizontal", t("ctx.recommend"), is_open(SUB_RECOMMEND), flip, rows, cx));
        let t_dl = tr.clone();
        if app.is_downloaded(&tr) {
            body.push(
                danger_item("ctx-undl".into(), "trash-2", t("ctx.remove_download"), cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.begin_ctx_close();
                        this.remove_downloads(vec![t_dl.clone()], cx)
                    }))
                    .into_any_element(),
            );
        } else if !app.is_downloading(&tr) {
            body.push(
                ctx_item("ctx-dl".into(), "download", t("ctx.download"), white(0.7), true, cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.begin_ctx_close();
                        this.download_tracks(vec![t_dl.clone()], cx)
                    }))
                    .into_any_element(),
            );
        }
        if let Some((l, i)) = m.from.clone().filter(|(l, _)| matches!(l, ListRef::User(_) | ListRef::Recent)) {
            let label = if l == ListRef::Recent { t("ctx.remove_recent") } else { t("ctx.remove") };
            let recent_track = (l == ListRef::Recent).then(|| tr.clone());
            body.push(
                danger_item("ctx-remove".into(), "circle-minus", label, cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.begin_ctx_close();
                        match &recent_track {
                            Some(track) => this.remove_recent_track(track, cx),
                            None => this.remove_shown(&l, i, cx),
                        }
                    }))
                    .into_any_element(),
            );
        }
        body.push(sep().into_any_element());
        let artist = tr.artist.clone();
        body.push(
            ctx_item("ctx-artist".into(), "user-round", t("ctx.artist"), white(0.7), true, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.begin_ctx_close();
                    this.open_artist(&artist, cx)
                }))
                .into_any_element(),
        );
        if let Some(album) = tr.album.clone().filter(|a| !a.trim().is_empty()) {
            let seed = tr.clone();
            body.push(
                ctx_item("ctx-album".into(), "disc-3", t("ctx.album"), white(0.7), true, cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.begin_ctx_close();
                        this.open_album(&album, &seed.artist, seed.clone(), cx)
                    }))
                    .into_any_element(),
            );
        }
        let t5 = tr.clone();
        body.push(
            ctx_item("ctx-link".into(), "link-2", t("ctx.copy_link"), white(0.7), true, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.begin_ctx_close();
                    this.share(&t5, cx);
                    cx.notify();
                }))
                .into_any_element(),
        );
    }

    // Opened on a sidebar entry: it can be hidden there.
    if let Some(id) = m.sidebar.clone() {
        if !body.is_empty() {
            body.push(sep().into_any_element());
        }
        body.push(
            ctx_item("ctx-side-hide".into(), "eye-off", t("ctx.hide_sidebar"), white(0.7), true, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.begin_ctx_close();
                    this.toggle_sidebar_hidden(&id, cx)
                }))
                .into_any_element(),
        );
    }
    if touch {
        let sub = SHEET_SUB.with(|s| s.borrow_mut().take());
        return Some(sheet(app, &m, body, sub, alpha, live, window, cx));
    }
    let measure = gpui::canvas(|b, _, _| MENU_BOUNDS.with(|m| m.set(Some(b))), |_, _, _, _| {}).absolute().inset_0();
    let menu = div()
        .id("ctx-menu")
        .relative()
        .child(measure)
        .w(px(MENU_W))
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
        .opacity(alpha)
        .mt(px(-6.0 * (1.0 - alpha)))
        .children(body);
    let layer = div().id("ctx-backdrop").absolute().inset_0();
    let layer = if live {
        // Nothing under the open menu reacts, not even hover.
        layer
            .occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.close_ctx(cx)))
            .on_mouse_down(MouseButton::Right, cx.listener(|this, _, _, cx| this.close_ctx(cx)))
            .on_scroll_wheel(cx.listener(|this, _, _, cx| this.close_ctx(cx)))
            .child(anchored().position(m.pos).snap_to_window_with_margin(px(8.0)).child(menu))
    } else {
        // Fading out: drawn only; the page behind takes clicks again.
        layer.child(anchored().position(m.pos).snap_to_window_with_margin(px(8.0)).child(menu))
    };
    Some(deferred(layer).with_priority(3).into_any_element())
}

thread_local! {
    /// Phones: the menu the sheet's pull belongs to (a new menu starts unpulled).
    static SHEET_OPENED: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
}

/// Swipe key of the sheet's pull-down (see `compact::pull`).
const SHEET_PULL: &str = "ctx-sheet-pull";

/// Phones: the menu as a bottom sheet (song header, big rows), over a dimmed
/// page; a tap outside or pulling it down closes it. An open submenu (`sub`)
/// takes the whole sheet: its title with a back arrow, then its rows.
#[allow(clippy::too_many_arguments)]
fn sheet(
    app: &mut Glyph,
    m: &crate::app::CtxMenu,
    body: Vec<AnyElement>,
    sub: Option<(SharedString, Vec<AnyElement>)>,
    alpha: f32,
    live: bool,
    window: &mut Window,
    cx: &mut Context<Glyph>,
) -> AnyElement {
    let vh = f32::from(window.viewport_size().height);
    let safe = window.insets().safe_area;
    let (sub_title, body) = match sub {
        Some((title, rows)) => (Some(title), rows),
        None => (None, body),
    };
    let back = sub_title.map(|title| {
        div()
            .id("ctx-sub-back")
            .flex()
            .items_center()
            .gap(px(14.0))
            .h(px(50.0))
            .px(px(10.0))
            .mb(px(4.0))
            .border_b_1()
            .border_color(white(0.08))
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                if let Some(m) = this.ctx_menu.as_mut() {
                    m.playlists = false;
                    this.ctx_opened = std::time::Instant::now();
                    crate::ui::widgets::handle_for("ctx-sheet").set_offset(gpui::point(px(0.0), px(0.0)));
                    cx.notify();
                }
            }))
            .child(icon("arrow-left", 20.0, white(0.9)))
            .child(div().flex_1().truncate().text_size(px(16.0)).font_weight(FontWeight::SEMIBOLD).text_color(white(0.95)).child(title))
            .into_any_element()
    });
    // A single song: cover, title and artist on top, like Spotify.
    let header = (back.is_none() && m.artist.is_none() && m.list.is_none() && m.sidebar.is_none()).then(|| {
        let tr = m.track.clone();
        div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .px(px(10.0))
            .pb(px(10.0))
            .mb(px(4.0))
            .border_b_1()
            .border_color(white(0.08))
            .child(cover(app, &tr, 48.0, 6.0, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .gap(px(2.0))
                    .child(div().truncate().text_size(px(15.0)).font_weight(FontWeight::SEMIBOLD).text_color(white(0.95)).child(tr.title.clone()))
                    .child(div().truncate().text_size(px(13.0)).text_color(white(0.55)).child(tr.artist.clone())),
            )
    });
    let h = crate::ui::widgets::handle_for("ctx-sheet");
    if SHEET_OPENED.with(|c| c.replace(Some(app.ctx_opened))) != Some(app.ctx_opened) {
        crate::ui::compact::swipe_reset(SHEET_PULL);
    }
    // Follows the finger while it is down (also when it rests), eases back
    // when let go short of closing, stays put while a closing sheet fades.
    let drag = crate::ui::compact::swipe_offset(SHEET_PULL, window).1.max(0.0);
    let panel = div()
        .id("ctx-sheet")
        .absolute()
        .left_0()
        .right_0()
        .bottom(px(-60.0 * (1.0 - alpha) - drag))
        .max_h(px(vh * 0.8))
        .track_scroll(&h)
        .overflow_y_scroll().touch_axis()
        .occlude()
        .rounded_t(px(16.0))
        .bg(gpui::hsla(0.0, 0.0, 0.1, 1.0))
        .px(px(8.0))
        .pt(px(8.0))
        .pb(safe.bottom + px(12.0))
        .flex()
        .flex_col()
        .font_family(theme::FONT)
        .opacity(alpha)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        // Pulled down from the top of its list: follows the finger, closes
        // when let go far enough down.
        .when(live, |d| d.on_scroll_wheel(crate::ui::compact::pull(SHEET_PULL, Some(h.clone()), |this, cx| this.close_ctx(cx), cx)))
        // Grab handle.
        .child(div().flex().justify_center().pb(px(10.0)).child(div().w(px(36.0)).h(px(4.0)).rounded_full().bg(white(0.25))))
        .children(back)
        .children(header)
        .children(body);
    // The long press that opened the menu may still be held: dragging on from
    // there pulls the sheet too (those moves stay with the long press and
    // never reach the sheet as a pan).
    let weak = cx.entity().downgrade();
    let follow = gpui::canvas(
        |_, _, _| {},
        move |_, _, w, _| {
            w.on_mouse_event(move |e: &gpui::LongPressEvent, phase, w, cx| {
                if !phase.bubble() {
                    return;
                }
                match e.phase {
                    gpui::TouchPhase::Moved => {
                        crate::ui::compact::swipe_follow(SHEET_PULL, (0.0, f32::from(e.position.y - e.start_position.y)));
                        w.refresh();
                    }
                    gpui::TouchPhase::Ended | gpui::TouchPhase::Cancelled => {
                        if crate::ui::compact::swipe_let_go(SHEET_PULL) {
                            weak.update(cx, |this, cx| this.close_ctx(cx)).ok();
                        }
                        w.refresh();
                    }
                    _ => {}
                }
            });
        },
    )
    .absolute()
    .size_0();
    let layer = div().id("ctx-backdrop").absolute().inset_0().bg(black(0.5 * alpha));
    let layer = if live {
        layer
            .child(follow)
            .occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.close_ctx(cx)))
            .child(panel)
    } else {
        layer.child(panel)
    };
    deferred(layer).with_priority(3).into_any_element()
}
