use super::*;

// ------------------------------------------------------------------ list view

pub fn list_view(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    let Some(l) = app.open_list.clone() else { return div().into_any_element() };
    // One artist's liked songs read like a "Show all" page: no cover column,
    // the play button sits next to the heading.
    if let ListRef::ArtistLiked(name) = &l {
        return liked_by_artist(app, name.clone(), cx);
    }
    let tracks = app.list_tracks(&l);
    let title = app.list_title(&l);
    let art = list_cover(app, &l, 248.0, cx);
    let art: AnyElement = match &l {
        ListRef::User(id) => {
            let id = id.clone();
            div()
                .id("list-cover-edit")
                .relative()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, w, cx| this.open_name_dialog(Vec::new(), Some(id.clone()), w, cx)))
                .child(art)
                // Veil and label fade in and out with the mouse.
                .child(hover_fill("list-cover", black(0.55), 12.0))
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(8.0))
                        .opacity(hover_k("list-cover"))
                        .child(icon("pencil", 28.0, white(0.95)))
                        .child(div().text_size(theme::text_sm()).text_color(white(0.95)).child(t("list.edit"))),
                )
                .into_any_element()
        }
        _ => art,
    };
    // Public playlist from a profile: its author shows as `[@handle]` by the title.
    let author = match &l {
        ListRef::Album { artist, .. } if artist.starts_with('@') => Some(artist.clone()),
        _ => None,
    };
    let kind = match &l {
        _ if app.shared_preview.is_some() => t("list.kind.playlist"),
        ListRef::Album { .. } if author.is_none() => t("list.kind.album"),
        _ => t("list.kind.playlist"),
    };
    // Albums and "liked by artist" show the artist, shared playlists their author.
    let sub: Option<AnyElement> = match &l {
        _ if app.shared_preview.is_some() => None,
        ListRef::Album { artist, .. } | ListRef::ArtistLiked(artist) if !artist.starts_with('@') => Some(
            div()
                .flex()
                .text_size(theme::text_sm())
                .text_color(theme::text_dim())
                .child(artist_link("list-artist", artist, cx))
                .into_any_element(),
        ),
        // Opened from a short link: who shared it.
        ListRef::User(id) => app.library.playlist(id).and_then(|p| p.shared_by.clone()).map(|h| {
            div()
                .flex()
                .text_size(theme::text_sm())
                .text_color(theme::text_dim())
                .child(tf("list.shared_by", &[("handle", &h)]).to_string())
                .into_any_element()
        }),
        _ => None,
    };
    let (t_play, t_shuf, t_queue) = (tracks.clone(), tracks.clone(), tracks.clone());
    let has = !tracks.is_empty();
    // Albums (and artist collections) can be saved to the library with the heart.
    let saveable = matches!(l, ListRef::Album { .. });
    let saved = app.shared_preview.is_none() && app.is_saved(&l);
    let l_save = l.clone();
    let user_id = match &l {
        ListRef::User(id) => Some(id.clone()),
        _ => None,
    };

    let l_more = l.clone();
    let all_dl = has && tracks.iter().all(|tr| app.is_downloaded(tr));
    let is_liked_list = l == ListRef::Liked;
    let dl_view = if has { app.downloads_view(&tracks) } else { crate::ui::widgets::DlView::None };
    let dl_tip = if has { app.download_tip(&tracks) } else { t("tip.download") };
    let t_dl = tracks.clone();
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
                // Type label sits tight on the title.
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(mono().text_size(px(11.0)).line_height(px(14.0)).text_color(theme::text_faint()).child(format!("[{}]", kind.to_uppercase())))
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_baseline()
                                .gap_x(px(8.0))
                                .min_w_0()
                                // Long names wrap inside the column (at most
                                // four lines) instead of running over the songs.
                                .child(
                                    div()
                                        .w_full()
                                        .min_w_0()
                                        .line_clamp(4)
                                        .text_size(px(22.0))
                                        .line_height(px(28.0))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme::text())
                                        .child(title.clone()),
                                )
                                .when_some(author.clone(), |d, h| {
                                    let h2 = h.clone();
                                    d.child(
                                        mono()
                                            .id("list-author")
                                            .text_size(theme::text_sm())
                                            .text_color(theme::text_faint())
                                            .cursor_pointer()
                                            .hover(|s| s.text_color(theme::text()))
                                            .child(format!("[{h}]"))
                                            .on_click(cx.listener(move |this, _, _, cx| this.open_profile(&h2, cx))),
                                    )
                                }),
                        ),
                )
                // Author and song count share one row to keep the stack short.
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_x(px(8.0))
                        .children(sub)
                        .child(meta_line(&tracks, theme::text_faint())),
                ),
        )
        .when(has, |d| {
            d.child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child({
                        // Playing from this list: the pill pauses / resumes it.
                        let here = app.playing_from.as_ref() == Some(&l)
                            && app.current.as_ref().is_some_and(|c| {
                                let k = crate::app::track_key(c);
                                tracks.iter().any(|t| crate::app::track_key(t) == k)
                            });
                        let (playing, loading) = (here && app.is_playing(), here && app.is_loading());
                        let label = if playing { t("common.pause") } else { t("list.play_all") };
                        play_pill("list-play", label, play_glyph(loading, playing, 14.0, 36.0)).flex_1().on_click(cx.listener(move |this, _, _, cx| {
                            if here {
                                this.toggle_play(cx)
                            } else {
                                this.play_list(t_play.clone(), 0, cx)
                            }
                        }))
                    })
                    .child(round_icon_button("list-shuffle", "shuffle", 36.0).on_click(cx.listener(move |this, _, _, cx| this.play_shuffled(t_shuf.clone(), cx)))),
            )
        })
        // Secondary actions: download, queue, save / edit, more.
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .when(has, |d| {
                    // Pink check when everything is saved (click removes), a
                    // filling ring around the arrow while downloads run.
                    let btn = match crate::ui::widgets::download_badge(dl_view, 20.0, "download") {
                        Some(badge) => crate::ui::widgets::with_tip(div().id("list-download"), Some(dl_tip.clone()))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(30.0))
                            .rounded(px(8.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::hover()))
                            .active(|s| s.bg(theme::active()))
                            .child(badge),
                        None => crate::ui::widgets::with_tip(
                            crate::ui::widgets::icon_button_plain("list-download", "download", 16.0, theme::text_dim()),
                            Some(dl_tip.clone()),
                        ),
                    };
                    d.child(btn.on_click(cx.listener(move |this, _, _, cx| {
                        if is_liked_list {
                            this.set_liked_auto_download(!all_dl, cx);
                        }
                        if let Some(l) = this.open_list.clone() {
                            this.touch_list(&l, 0.5);
                        }
                        if all_dl {
                            this.remove_downloads(t_dl.clone(), cx)
                        } else {
                            this.download_tracks(t_dl.clone(), cx)
                        }
                    })))
                    .child(icon_button("list-queue", "list-plus", 16.0, theme::text_dim()).on_click(cx.listener(move |this, _, _, cx| this.queue_all(t_queue.clone(), cx))))
                })
                .when(app.shared_preview.is_some(), |d| {
                    let l2 = l_save.clone();
                    d.child(div().id("share-add-library").cursor_pointer().px(px(10.0)).py(px(6.0)).rounded(px(8.0))
                        .text_size(theme::text_sm()).child(t("list.add_shared").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_saved(&l2, cx))))
                })
                .when(saveable && app.shared_preview.is_none(), |d| {
                    let key: SharedString = "list-save".into();
                    let k = key.clone();
                    let l2 = l_save.clone();
                    d.child(toggle_button(key, "heart", 16.0, saved, Some(theme::pink())).on_click(cx.listener(move |this, _, _, cx| {
                        mark_toggled(k.clone());
                        this.toggle_saved(&l2, cx)
                    })))
                })
                .when_some(user_id, |d, id| {
                    let id2 = id.clone();
                    let adding = app.add_songs.as_ref().is_some_and(|a| a.open && a.id == id);
                    d.child(icon_button("list-edit", "pencil", 16.0, theme::text_dim()).on_click(cx.listener(move |this, _, w, cx| this.open_name_dialog(Vec::new(), Some(id.clone()), w, cx))))
                        // Side panel with suggested songs to add.
                        .child(
                            icon_button("list-add-songs", "circle-plus", 16.0, if adding { theme::pink() } else { theme::text_dim() })
                                .on_click(cx.listener(move |this, _, w, cx| this.toggle_add_songs(&id2, w, cx))),
                        )
                })
                .child(icon_button("list-more", "ellipsis-vertical", 16.0, theme::text_dim()).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.open_list_ctx(l_more.clone(), e.position, cx)
                    }),
                )),
        );

    // Recently played can hold artists without a single song; then the mixed
    // rows still render below.
    let empty = if matches!(l, ListRef::Recent) { app.library.recent.is_empty() } else { tracks.is_empty() };
    let right: AnyElement = if empty {
        let (a, b) = match &l {
            ListRef::Liked | ListRef::ArtistLiked(_) => (t("library.liked.empty"), t("library.liked.empty.sub")),
            ListRef::Recent => (t("home.empty.title"), t("library.recent.empty.sub")),
            ListRef::Downloads => (t("library.downloads.empty"), t("library.downloads.empty.sub")),
            ListRef::User(_) => (t("library.playlist.empty"), t("list.empty.sub.user")),
            _ => (t("library.playlist.empty"), t("list.empty.sub")),
        };
        ascii_state(ART_EMPTY, a, b).into_any_element()
    } else {
        let list = Rc::new(tracks);
        let show_album = !matches!(l, ListRef::Album { .. });
        let scroll = app.main_scroll.clone();
        // Recently played mixes songs and artists; the rows follow that order.
        let recent = matches!(l, ListRef::Recent).then(|| {
            let items = app.library.recent.clone();
            let mut songs: Vec<Track> = Vec::new();
            let mut song_at: Vec<usize> = Vec::with_capacity(items.len());
            for it in &items {
                match it {
                    RecentItem::Song { track } => {
                        song_at.push(songs.len());
                        songs.push(track.clone());
                    }
                    RecentItem::Artist { .. } => song_at.push(usize::MAX),
                }
            }
            (Rc::new(items), Rc::new(songs), Rc::new(song_at))
        });
        let n = recent.as_ref().map_or(list.len(), |(items, ..)| items.len());
        // Own playlists: songs are dragged into a new order; the rows in
        // between glide aside (same feel as the queue).
        let own = match &l {
            ListRef::User(id) => Some(id.clone()),
            _ => None,
        };
        if !cx.has_active_drag() {
            app.list_drop = None;
        }
        let drop = app.list_drop.filter(|_| own.is_some());
        let rows = crate::ui::widgets::virtual_rows("list-row", n, LIST_PITCH, &scroll, |i| {
            if let Some((items, songs, song_at)) = &recent {
                if let RecentItem::Artist { name, track } = &items[i] {
                    return recent_artist_row(app, i, name, track.as_ref(), cx);
                }
                return song_row(app, songs.clone(), song_at[i], "list-row", Some(l.clone()), show_album, cx).into_any_element();
            }
            let row = song_row(app, list.clone(), i, "list-row", Some(l.clone()), show_album, cx);
            let Some(id) = own.clone() else { return row.into_any_element() };
            let art = app.cover_path(&list[i], cx);
            let drag = SongDrag { list: id, from: i, track: list[i].clone(), art };
            let lifted = drop.is_some_and(|(from, _)| from == i);
            let shift = drag_shift("list-row", i, drop, LIST_PITCH);
            div()
                .relative()
                .h(px(LIST_PITCH))
                .child(
                    row.on_drag(drag, |d, _, _, cx| cx.new(|_| d.clone()))
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(px(shift))
                        .when(lifted, |r| r.opacity(0.0)),
                )
                .into_any_element()
        });
        match own {
            Some(id) => rows
                .on_drag_move::<SongDrag>(cx.listener(move |this, e: &gpui::DragMoveEvent<SongDrag>, _, cx| {
                    let d = e.drag(cx);
                    if d.list != id {
                        return;
                    }
                    let y = f32::from(e.event.position.y - e.bounds.top());
                    let slot = ((y / LIST_PITCH).floor().max(0.0) as usize).min(n.saturating_sub(1));
                    let from = d.from;
                    if this.list_drop != Some((from, slot)) {
                        this.list_drop = Some((from, slot));
                        cx.notify();
                    }
                }))
                .on_drop(cx.listener(|this, d: &SongDrag, _, cx| {
                    let to = this.list_drop.take().map_or(d.from, |(_, slot)| slot);
                    let id = d.list.clone();
                    this.move_in_playlist(&id, d.from, to, cx);
                }))
                .into_any_element(),
            None => rows.into_any_element(),
        }
    };

    // Own playlists take music files / folders dropped from the file manager.
    let drop_id = match &l {
        ListRef::User(id) => Some(id.clone()),
        _ => None,
    };

    // Two columns side by side, each scrolling on its own: the songs run up
    // to the top edge (they fade out under the title bar instead of being
    // cut off below the back button), and the cover / info column scrolls
    // when the window is too low to show all of its actions.
    let back = back_button(app, cx);
    let top = 20.0 + if back.is_some() { 30.0 + 16.0 } else { 0.0 };
    let left_scroll = crate::ui::widgets::handle_for("list-left");
    let left_col = div()
        .relative()
        .flex_none()
        .w(px(248.0))
        .h_full()
        .child(
            div()
                .id("list-left")
                .track_scroll(&left_scroll)
                .size_full()
                .overflow_y_scroll()
                .pt(px(20.0))
                .pb(px(24.0))
                .flex()
                .flex_col()
                .gap(px(16.0))
                .children(back.map(|b| div().flex_none().flex().child(b)))
                .child(left),
        )
        .child(crate::ui::widgets::smooth_wheel(&left_scroll, false, cx));
    div()
        .size_full()
        .flex()
        .gap(px(36.0))
        .px(px(32.0))
        .when_some(drop_id, |d, id| {
            d.drag_over::<ExternalPaths>(|s, _, _, _| s.bg(theme::hover()))
                .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                    this.import_files_to_playlist(id.clone(), paths.paths().to_vec(), cx)
                }))
        })
        .child(left_col)
        .child(
            div()
                .id("list-scroll")
                .track_scroll(&app.main_scroll)
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scroll()
                .pt(px(top))
                .pb(px(20.0))
                .child(right),
        )
        .into_any_element()
}

/// "Liked songs" of one artist: back button, round play button left of the
/// heading, then the songs — the shape of a "Show all" page (no cover column).
pub(super) fn liked_by_artist(app: &mut Glyph, name: String, cx: &mut Context<Glyph>) -> AnyElement {
    let l = ListRef::ArtistLiked(name.clone());
    let tracks = app.list_tracks(&l);
    let back = back_button(app, cx);
    // One of these songs is on: the button pauses / resumes it.
    let here = app.current.as_ref().is_some_and(|c| {
        let k = crate::app::track_key(c);
        tracks.iter().any(|t| crate::app::track_key(t) == k)
    });
    let (playing, loading) = (here && app.is_playing(), here && app.is_loading());
    let t_play = tracks.clone();
    let play = (!tracks.is_empty()).then(|| {
        round_play("liked-play", playing, loading, 44.0).on_click(cx.listener(move |this, _, _, cx| {
            if here {
                this.toggle_play(cx)
            } else {
                this.play_list(t_play.clone(), 0, cx)
            }
        }))
    });
    let body: AnyElement = if tracks.is_empty() {
        ascii_state(ART_EMPTY, t("library.liked.empty"), t("library.liked.empty.sub")).into_any_element()
    } else {
        let list = Rc::new(tracks.clone());
        let scroll = app.main_scroll.clone();
        let n = list.len();
        crate::ui::widgets::virtual_rows("list-row", n, LIST_PITCH, &scroll, |i| {
            song_row(app, list.clone(), i, "list-row", Some(l.clone()), true, cx).into_any_element()
        })
        .into_any_element()
    };
    div()
        .size_full()
        .flex()
        .flex_col()
        .gap(px(16.0))
        .px(px(32.0))
        .pt(px(20.0))
        .children(back.map(|b| div().flex_none().flex().child(b)))
        // Heading and its info line sit close together (a tighter gap than the
        // page's), so the artist / song count reads as part of the title.
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(div().flex().items_center().gap(px(14.0)).children(play).child(heading(t("library.liked"))))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .child(artist_link("liked-artist", &name, cx))
                        .child(meta_line(&tracks, theme::text_faint())),
                ),
        )
        .child(
            div()
                .id("artist-liked-scroll")
                .track_scroll(&app.main_scroll)
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .pb(px(20.0))
                .child(body),
        )
        .into_any_element()
}
