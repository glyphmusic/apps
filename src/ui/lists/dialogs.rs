use super::*;
use gpui::Div;

// ------------------------------------------------------------------ name dialog

/// "Or paste a link" field of the New playlist dialog, with what the link
/// was recognised as, the import progress or its error underneath.
fn link_section(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let value = app.playlist_link.read(cx).value.trim().to_string();
    let found = glyph_core::import::parse_url(&value);
    let status: Option<Div> = match &app.link_import {
        Some(l) if l.error.is_some() => Some(
            div()
                .flex()
                .items_start()
                .gap(px(6.0))
                .text_size(theme::text_sm())
                .text_color(white(0.7))
                .child(div().flex_none().pt(px(2.0)).child(icon("info", 13.0, white(0.6))))
                .child(div().min_w_0().child(l.error.clone().unwrap_or_default())),
        ),
        Some(l) if l.total > 0 => {
            let p = glyph_core::ImportProgress { label: format!("IMPORT {}", l.label), done: l.done, total: l.total };
            Some(mono().text_size(theme::text_xs()).text_color(white(0.65)).child(glyph_core::import::format_progress(&p, 14)))
        }
        Some(_) => Some(spinner_line(app.spinner(), t("dialog.playlist.reading"))),
        None if value.is_empty() => Some(div().text_size(theme::text_sm()).text_color(white(0.45)).child(t("dialog.playlist.link.hint"))),
        None => Some(match &found {
            Some(src) => div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .text_size(theme::text_sm())
                .text_color(white(0.65))
                .child(icon("check", 13.0, white(0.65)))
                .child(tf("dialog.playlist.link.found", &[("kind", &src.label())])),
            None => div().text_size(theme::text_sm()).text_color(white(0.5)).child(t("import.unknown_link")),
        }),
    };
    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .h(px(38.0))
                .px(px(12.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .rounded(px(8.0))
                .bg(black(0.45))
                .border_1()
                .border_color(white(0.12))
                .text_size(px(13.5))
                .child(icon("link-2", 14.0, white(0.5)))
                .child(div().flex_1().min_w_0().child(app.playlist_link.clone())),
        )
        .children(status)
}

/// "New playlist" / "Rename playlist": a small centred dialog with one field.
pub fn name_dialog(app: &mut Glyph, cx: &mut Context<Glyph>) -> AnyElement {
    let editing = app.name_dialog.as_ref().and_then(|d| d.rename.clone());
    let renaming = editing.is_some();
    let eq = app.name_dialog.as_ref().is_some_and(|d| d.eq_preset);
    let input = app.playlist_name.clone();
    if let Some(id) = editing.clone() {
        return edit_dialog(app, id, cx);
    }
    let picture = editing.map(|id| {
        let l = ListRef::User(id.clone());
        let art = list_cover(app, &l, 112.0, cx);
        let has = app.library.playlist(&id).is_some_and(|p| p.cover.is_some());
        let (id1, id2) = (id.clone(), id.clone());
        div()
            .flex()
            .items_center()
            .gap(px(16.0))
            .child(
                div()
                    .id("dialog-cover")
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.pick_playlist_cover(id1.clone(), cx)))
                    .child(art),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(button("dialog-pick", t("dialog.playlist.pick")).on_click(cx.listener(move |this, _, _, cx| this.pick_playlist_cover(id.clone(), cx))))
                    .when(has, |d| {
                        d.child(
                            text_button("dialog-unpick", t("dialog.playlist.unpick"))
                                .text_size(theme::text_sm())
                                .on_click(cx.listener(move |this, _, _, cx| this.remove_playlist_cover(&id2, cx))),
                        )
                    }),
            )
    });
    let new_playlist = !eq && !renaming;
    let has_link = new_playlist && !app.playlist_link.read(cx).value.trim().is_empty();
    let importing = new_playlist && app.link_import.as_ref().is_some_and(|l| l.error.is_none());
    let link = new_playlist.then(|| link_section(app, cx));
    let ok_label = if renaming || eq {
        t("dialog.save")
    } else if importing {
        t("dialog.importing")
    } else if has_link {
        t("dialog.import")
    } else {
        t("dialog.create")
    };
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
                .child(if eq {
                    t("dialog.eq.title")
                } else if renaming {
                    t("dialog.playlist.rename")
                } else {
                    t("dialog.playlist.title")
                }),
        )
        .children(picture)
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
        .children(link)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .when(new_playlist, |d| {
                    d.child(
                        text_button("dialog-file", t("dialog.playlist.file"))
                            .text_size(theme::text_sm())
                            .when(!importing, |b| b.on_click(cx.listener(|this, _, _, cx| this.import_playlist_file(cx)))),
                    )
                })
                .child(div().flex_1())
                .child(button("dialog-cancel", t("dialog.cancel")).on_click(cx.listener(|this, _, _, cx| this.close_name_dialog(cx))))
                .child(
                    primary_button("dialog-ok", ok_label)
                        .when(importing, |b| b.opacity(0.5).cursor_default())
                        .when(!importing, |b| b.on_click(cx.listener(|this, _, _, cx| this.submit_name_dialog(cx)))),
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

/// "Edit details" of an own playlist: picture left (click to change), name
/// and per-playlist switches right, Save bottom right.
pub(super) fn edit_dialog(app: &mut Glyph, id: String, cx: &mut Context<Glyph>) -> AnyElement {
    if let Some(on) = app.profile.settings.as_ref().map(|s| s.playlists.contains(&id)) {
        if let Some(initial) = app.name_dialog.as_mut().and_then(|d| d.initial_playlist.as_mut()) {
            if initial.3.is_none() { initial.3 = Some(on); }
        }
    }
    let changed = app.playlist_edit_changed(cx);
    let l = ListRef::User(id.clone());
    let art = list_cover(app, &l, 176.0, cx);
    let has = app.library.playlist(&id).is_some_and(|p| p.cover.is_some());
    let auto = app.is_auto_download(&id);
    let on_profile = app.profile.settings.as_ref().map(|s| s.playlists.contains(&id));
    let (id1, id2, id3, id4) = (id.clone(), id.clone(), id.clone(), id.clone());
    let cover = div()
        .id("dialog-cover")
        .relative()
        .flex_none()
        .size(px(176.0))
        .rounded(px(8.0))
        .overflow_hidden()
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| this.pick_playlist_cover(id1.clone(), cx)))
        .child(art)
        // Hover: fades to dark with "choose picture".
        .child(hover_fill("dialog-cover", black(0.6), 8.0))
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .opacity(hover_k("dialog-cover"))
                .child(icon("image-plus", 28.0, white(0.95)))
                .child(div().text_size(theme::text_sm()).text_color(white(0.95)).child(t("dialog.playlist.pick"))),
        );
    let field = div()
        .h(px(40.0))
        .px(px(12.0))
        .flex()
        .items_center()
        .rounded(px(8.0))
        .bg(white(0.07))
        .text_size(px(14.0))
        .child(app.playlist_name.clone());
    let switches = div()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .pt(px(4.0))
        .child(toggle("dialog-auto-dl", auto, t("ctx.auto_download")).on_click(cx.listener(move |this, _, _, cx| this.toggle_auto_download(&id2, cx))))
        .when_some(on_profile, |d, on| {
            d.child(toggle("dialog-profile", on, t("dialog.playlist.on_profile")).on_click(cx.listener(move |this, _, _, cx| this.profile_toggle_playlist(id3.clone(), cx))))
        });
    let panel = div()
        .id("name-dialog")
        .occlude()
        .w(px(540.0))
        .p(px(24.0))
        .rounded(px(12.0))
        .bg(gpui::hsla(0.0, 0.0, 0.09, 0.99))
        .flex()
        .flex_col()
        .gap(px(18.0))
        .font_family(theme::FONT)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .flex()
                .items_center()
                .child(div().flex_1().text_size(px(20.0)).font_weight(FontWeight::BOLD).text_color(white(0.95)).child(t("dialog.playlist.rename")))
                .child(icon_button("dialog-close", "x", 18.0, white(0.6)).on_click(cx.listener(|this, _, _, cx| this.close_name_dialog(cx)))),
        )
        .child(div().flex().gap(px(16.0)).child(cover).child(div().flex_1().min_w_0().flex().flex_col().gap(px(12.0)).child(field).child(switches)))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .when(has, |d| {
                    d.child(
                        text_button("dialog-unpick", t("dialog.playlist.unpick"))
                            .text_size(theme::text_sm())
                            .on_click(cx.listener(move |this, _, _, cx| this.remove_playlist_cover(&id4, cx))),
                    )
                })
                .child(div().flex_1())
                .child(
                    primary_button("dialog-ok", t("dialog.save"))
                        .h(px(40.0))
                        .px(px(24.0))
                        .rounded(px(8.0))
                        .bg(if changed { gpui::white() } else { white(0.16) })
                        .text_color(if changed { gpui::black() } else { white(0.4) })
                        .when(changed, |d| d.on_click(cx.listener(|this, _, _, cx| this.submit_name_dialog(cx)))),
                ),
        );
    deferred(
        div()
            .id("dialog-backdrop")
            .absolute()
            .inset_0()
            .bg(black(0.55))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.close_name_dialog(cx)))
            .child(panel),
    )
    .with_priority(4)
    .into_any_element()
}
