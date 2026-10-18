//! Song lists (Liked, Recent, playlists, albums), the artist profile, the
//! library grid and the right-click song menu.
//!
//! Layout follows the usual desktop-player pattern (cover column + numbered
//! list; artist hero + popular songs) with Glyph's own touches: the artist
//! hero is the photo rendered as coloured ASCII, and counts / durations are
//! set in mono like terminal output.

use crate::app::{Glyph, ListRef};
use crate::i18n::{t, tf};
use crate::theme;
use crate::ui::widgets::*;
use glyph_protocol::Track;
use gpui::{
    anchored, deferred, div, img, linear_color_stop, linear_gradient, prelude::*, px, AnyElement, Context, ElementId, ExternalPaths,
    FontWeight, Hsla, MouseButton, MouseDownEvent, ObjectFit, SharedString, Stateful, Window,
};
use std::rc::Rc;

fn white(a: f32) -> Hsla {
    gpui::hsla(0.0, 0.0, 1.0, a)
}

fn black(a: f32) -> Hsla {
    gpui::hsla(0.0, 0.0, 0.0, a)
}

/// "11:50" or "1:02:03" for a list's total length.
fn total_time(tracks: &[Track]) -> String {
    let s: u64 = tracks.iter().filter_map(|t| t.duration_ms).sum::<u64>() / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Blue check badge for verified artists (drawn, no icon file needed).
pub fn verified_badge(size: f32) -> gpui::Div {
    div().flex_none().child(with_tip(div().id("verified-badge"), Some(t("tip.verified")))
        .flex_none()
        .size(px(size))
        .rounded_full()
        .bg(gpui::hsla(212.0 / 360.0, 0.9, 0.56, 1.0))
        .flex()
        .items_center()
        .justify_center()
        .child(icon("check", size * 0.62, gpui::white())))
}

/// 1234 -> "1.2K", 3_400_000 -> "3.4M".
pub fn compact_count(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}K", n as f64 / 1e3).replace(".0K", "K"),
        _ => format!("{:.1}M", n as f64 / 1e6).replace(".0M", "M"),
    }
}

/// `[ 07 SONGS · 11:50 ]`-style meta line (mono, the ASCII accent).
fn meta_line(tracks: &[Track], color: Hsla) -> gpui::Div {
    mono()
        .text_size(px(11.5))
        .text_color(color)
        .child(format!("[ {:02} {} · {} ]", tracks.len(), t("list.songs_caps"), total_time(tracks)))
}

/// Floating "Back" pill (only when there is history).
pub(crate) fn back_button(app: &Glyph, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    (!app.history.is_empty()).then(|| {
        div()
            .id("nav-back")
            // Only as wide as its label, also inside column layouts.
            .self_start()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.0))
            .h(px(30.0))
            .px(px(12.0))
            .rounded(px(8.0))
            .bg(black(0.55))
            .border_1()
            .border_color(white(0.1))
            .cursor_pointer()
            .text_size(theme::text_sm())
            .text_color(white(0.9))
            .hover(|s| s.bg(black(0.75)))
            .on_click(cx.listener(|this, _, _, cx| this.go_back(cx)))
            .child(icon("arrow-left", 14.0, white(0.9)))
            .child(t("common.back"))
            .into_any_element()
    })
}

/// Clickable artist name: opens the artist profile.
pub fn artist_link(id: impl Into<ElementId>, name: &str, cx: &mut Context<Glyph>) -> Stateful<gpui::Div> {
    artist_link_label(id, name, name.to_string(), cx)
}

/// A temporary display label never changes the artist the link/menu opens.
pub fn artist_link_label(id: impl Into<ElementId>, name: &str, label: String, cx: &mut Context<Glyph>) -> Stateful<gpui::Div> {
    let n = name.to_string();
    with_artist_menu(div().id(id), name.to_string(), cx)
        .min_w_0()
        .truncate()
        .cursor_pointer()
        .hover(|s| s.underline())
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.open_artist(&n, cx)
        }))
        .child(label)
}

/// Clickable album name: opens the album page.
pub fn album_link(id: impl Into<ElementId>, tr: &Track, cx: &mut Context<Glyph>) -> Option<Stateful<gpui::Div>> {
    let title = tr.album.clone().filter(|a| !a.trim().is_empty())?;
    let seed = tr.clone();
    let t2 = title.clone();
    Some(
        div()
            .id(id)
            .min_w_0()
            .truncate()
            .cursor_pointer()
            .hover(|s| s.underline())
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.open_album(&t2, &seed.artist, seed.clone(), cx)
            }))
            .child(title),
    )
}

/// Right-click on an artist (name, tile, row) opens the artist menu.
pub fn with_artist_menu(el: Stateful<gpui::Div>, name: String, cx: &mut Context<Glyph>) -> Stateful<gpui::Div> {
    el.on_mouse_down(
        MouseButton::Right,
        cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            this.open_artist_ctx(&crate::app::primary_artist(&name), e.position, cx)
        }),
    )
}

/// Right-click on an album / playlist tile opens the list menu.
pub fn with_list_menu(el: Stateful<gpui::Div>, l: ListRef, cx: &mut Context<Glyph>) -> Stateful<gpui::Div> {
    el.on_mouse_down(
        MouseButton::Right,
        cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            this.open_list_ctx(l.clone(), e.position, cx)
        }),
    )
}

/// Right-click on a song opens the song menu.
pub fn with_menu(el: Stateful<gpui::Div>, tr: Track, from: Option<(ListRef, usize)>, cx: &mut Context<Glyph>) -> Stateful<gpui::Div> {
    el.on_mouse_down(
        MouseButton::Right,
        cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            this.open_ctx(tr.clone(), e.position, from.clone(), cx)
        }),
    )
}

// ------------------------------------------------------------------ covers

/// Cover for a list: heart tile (Liked), clock tile (Recent), 2×2 mosaic or
/// single cover (playlists), first cover (albums).
pub fn list_cover(app: &mut Glyph, l: &ListRef, size: f32, cx: &mut Context<Glyph>) -> AnyElement {
    let r = (size * 0.06).clamp(6.0, 12.0);
    let tile = |bg: Hsla, name: &str, fg: Hsla| {
        div()
            .flex_none()
            .size(px(size))
            .rounded(px(r))
            .bg(bg)
            .flex()
            .items_center()
            .justify_center()
            .child(icon(name, (size * 0.3).clamp(16.0, 72.0), fg))
            .into_any_element()
    };
    let tracks = app.list_tracks(l);
    // Own playlists and public ones from a profile (author `@handle`) get the
    // automatic mosaic from their songs' covers.
    let mosaic = matches!(l, ListRef::User(_)) || matches!(l, ListRef::Album { artist, .. } if artist.starts_with('@'));
    match l {
        ListRef::Liked | ListRef::ArtistLiked(_) => tile(theme::accent_tile(0.22, 0.45), "heart-fill", theme::pink()),
        ListRef::Recent => tile(theme::card(), "history", white(0.75)),
        ListRef::Downloads => tile(theme::accent_tile(0.16, 0.3), "download", theme::pink()),
        ListRef::User(id) if app.library.playlist(id).is_some_and(|p| p.cover.is_some()) => {
            let p = app.library.playlist(id).and_then(|p| p.cover.clone()).unwrap_or_default();
            img(std::path::PathBuf::from(p)).flex_none().size(px(size)).rounded(px(r)).object_fit(ObjectFit::Cover).into_any_element()
        }
        _ if mosaic => {
            // Distinct covers (by path) for the mosaic.
            let mut paths: Vec<std::path::PathBuf> = Vec::new();
            for tr in tracks.iter().take(12) {
                if let Some(p) = app.cover_path(tr, cx) {
                    if !paths.contains(&p) {
                        paths.push(p);
                    }
                }
                if paths.len() == 4 {
                    break;
                }
            }
            if paths.len() == 4 {
                let h = size / 2.0;
                let rr = px(r);
                let q = |p: std::path::PathBuf| img(p).size(px(h)).object_fit(ObjectFit::Cover);
                let mut it = paths.into_iter();
                let (a, b, c, d) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
                div()
                    .flex_none()
                    .size(px(size))
                    .flex()
                    .flex_col()
                    .child(div().flex().child(q(a).rounded_tl(rr)).child(q(b).rounded_tr(rr)))
                    .child(div().flex().child(q(c).rounded_bl(rr)).child(q(d).rounded_br(rr)))
                    .into_any_element()
            } else if paths.len() >= 2 {
                // Automatic thumbnail from the songs' covers: halves, or one
                // big + two small.
                let rr = px(r);
                let img_ = |p: std::path::PathBuf, w: f32, h: f32| img(p).w(px(w)).h(px(h)).object_fit(ObjectFit::Cover);
                let mut it = paths.into_iter();
                let a = it.next().unwrap();
                let b = it.next().unwrap();
                match it.next() {
                    None => div()
                        .flex_none()
                        .size(px(size))
                        .flex()
                        .child(img_(a, size / 2.0, size).rounded_l(rr))
                        .child(img_(b, size / 2.0, size).rounded_r(rr))
                        .into_any_element(),
                    Some(c) => div()
                        .flex_none()
                        .size(px(size))
                        .flex()
                        .child(img_(a, size / 2.0, size).rounded_l(rr))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(img_(b, size / 2.0, size / 2.0).rounded_tr(rr))
                                .child(img_(c, size / 2.0, size / 2.0).rounded_br(rr)),
                        )
                        .into_any_element(),
                }
            } else if let Some(first) = tracks.first() {
                cover(app, first, size, r, cx).into_any_element()
            } else {
                tile(theme::card(), "list-music", white(0.5))
            }
        }
        // `User` always takes the mosaic arm above.
        ListRef::Album { .. } | ListRef::User(_) => match tracks.first() {
            Some(first) => cover(app, first, size, r, cx).into_any_element(),
            None => tile(theme::card(), "disc-3", white(0.5)),
        },
    }
}

// ------------------------------------------------------------------ song rows

thread_local! {
    /// Width of the song rows per list (measured on the first row).
    static ROW_W: std::cell::RefCell<std::collections::HashMap<&'static str, f32>> = std::cell::RefCell::new(Default::default());
}

/// Rows narrower than this drop the album column, so title and artist keep
/// their room (the album stays in the song menu).
const ALBUM_COL_MIN_ROW: f32 = 700.0;

/// Numbered song row: # (play on hover) · cover · title / artist · album ·
/// heart · time · ⋮. Click plays the list from here; right-click or ⋮ opens
/// the song menu.
pub fn song_row(
    app: &mut Glyph,
    list: Rc<Vec<Track>>,
    i: usize,
    prefix: &'static str,
    from: Option<ListRef>,
    show_album: bool,
    cx: &mut Context<Glyph>,
) -> Stateful<gpui::Div> {
    let tr = list[i].clone();
    let playing = app.current.as_ref().map(crate::app::track_key) == Some(crate::app::track_key(&tr));
    let liked = app.is_liked(&tr);
    // Hidden explicit song: shown dimmed, does not play.
    let blocked = !app.allowed(&tr);
    let dl_badge = crate::ui::widgets::download_badge(app.download_view(&tr), 14.0, "arrow-down");
    let group: SharedString = format!("{prefix}-g-{i}").into();
    let art = cover(app, &tr, 40.0, 6.0, cx);
    let l2 = list.clone();
    let from_ctx = from.clone().map(|l| (l, i));
    let (t_like, t_menu) = (tr.clone(), tr.clone());
    let from_menu = from_ctx.clone();
    let t_hover = tr.clone();
    let hk = hover_k(&group);
    let show_album = show_album && ROW_W.with(|m| m.borrow().get(prefix).copied()).is_none_or(|w| w >= ALBUM_COL_MIN_ROW);
    let row = div()
        .id((prefix, i))
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| this.prefetch_on_hover(&t_hover, *hovered, cx)))
        .flex()
        .items_center()
        .gap(px(14.0))
        .h(px(56.0))
        .px(px(10.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .when(blocked, |d| d.opacity(0.4))
        .relative()
        .child(hover_fill(group.clone(), white(0.05), 8.0))
        .when(i == 0, |d| {
            d.child(
                gpui::canvas(
                    move |b, window, _| {
                        let w = f32::from(b.size.width);
                        let old = ROW_W.with(|m| m.borrow_mut().insert(prefix, w));
                        // Crossing the limit shows / hides the album column.
                        if old.is_none_or(|o| (o >= ALBUM_COL_MIN_ROW) != (w >= ALBUM_COL_MIN_ROW)) {
                            window.request_animation_frame();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
        })
        .on_click(cx.listener(move |this, _, _, cx| this.play_list((*l2).clone(), i, cx)))
        .child(
            div()
                .flex_none()
                .w(px(26.0))
                .h(px(20.0))
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    mono()
                        .text_size(theme::text_sm())
                        .text_color(if playing { theme::pink() } else { theme::text_faint() })
                        .opacity(1.0 - hk)
                        .child(format!("{}", i + 1)),
                )
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(hk)
                        .child(icon("play", 13.0, theme::text())),
                ),
        )
        .child(art)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                // Title and artist keep this much before anything else gives.
                .min_w(px(160.0))
                .gap(px(2.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .min_w_0()
                        .when(tr.explicit, |d| d.child(explicit_badge()))
                        .child(
                            div()
                                .truncate()
                                .text_size(theme::text_md())
                                .font_weight(if playing { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                                .text_color(if playing { theme::pink() } else { theme::text() })
                                .child(tr.title.clone()),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .min_w_0()
                        .text_size(theme::text_sm())
                        .text_color(theme::text_dim())
                        .children(dl_badge)
                        .child(artist_link((prefix, i * 8 + 1), &tr.artist, cx)),
                ),
        )
        .when(show_album, |d| {
            d.child(
                div()
                    .flex_shrink(1.0)
                    .min_w_0()
                    .w(px(200.0))
                    .truncate()
                    .text_size(theme::text_sm())
                    .text_color(theme::text_faint())
                    .flex()
                    .children(album_link((prefix, i * 8 + 3), &tr, cx)),
            )
        })
        .child({
            let key: SharedString = format!("{prefix}-like-{}", crate::app::track_key(&t_like)).into();
            let k = key.clone();
            div()
                .flex_none()
                .when(!liked, |d| d.opacity(hk))
                .child(toggle_button(key, "heart", 15.0, liked, Some(theme::pink())).on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    mark_toggled(k.clone());
                    this.toggle_like(&t_like, cx)
                })))
        })
        .child(
            mono()
                .flex_none()
                .w(px(40.0))
                .text_size(theme::text_sm())
                .text_color(theme::text_faint())
                .child(fmt_ms(tr.duration_ms)),
        )
        .child(
            div()
                .id((prefix, i * 8 + 2))
                .flex_none()
                .size(px(28.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .hover(|s| s.bg(white(0.08)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.open_ctx(t_menu.clone(), e.position, from_menu.clone(), cx)
                    }),
                )
                .child(icon("ellipsis-vertical", 16.0, theme::text_dim())),
        );
    with_menu(row, tr, from_ctx, cx)
}

/// "Follow" / "Following" outline button for an artist.
pub fn follow_button(id: impl Into<ElementId>, name: &str, track: Option<Track>, app: &Glyph, h: f32, cx: &mut Context<Glyph>) -> Stateful<gpui::Div> {
    let following = app.is_following(name);
    let n = name.to_string();
    let id: ElementId = id.into();
    let key = format!("follow-{id:?}");
    let swap = format!("follow-swap:{n}");
    let k = reveal_peek(&swap, 0.45);
    let e = 1.0 - (1.0 - k).powi(3);
    let label = if following { t("artist.following") } else { t("artist.follow") };
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .h(px(h))
        .px(px(16.0))
        .rounded(px(8.0))
        .border_1()
        // Following: quiet like the icon buttons next to it; not yet: a
        // brighter outline that asks for the click.
        .border_color(if following { white(0.14) } else { white(0.45) })
        .bg(black(0.35))
        .cursor_pointer()
        .text_size(theme::text_sm())
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(white(0.95))
        .relative()
        .child(hover_fill(key, white(0.1), 8.0))
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            reveal_restart(&format!("follow-swap:{n}"));
            this.toggle_follow(&n, track.clone(), cx)
        }))
        // The check grows in (or shrinks away) while the label decodes.
        .when(following || k < 1.0, |d| {
            let w = if following { e } else { 1.0 - e };
            d.child(
                div()
                    .flex_none()
                    .w(px(20.0 * w))
                    .overflow_hidden()
                    .opacity(w)
                    .child(icon("check", 14.0 * (0.6 + 0.4 * w), white(0.95))),
            )
        })
        .child(decode(&label, k))
}

fn round_icon_button(id: &'static str, name: &str, size: f32) -> Stateful<gpui::Div> {
    let tip = tip_for(name);
    with_tip(div().id(id), tip)
        .flex_none()
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.0))
        .bg(black(0.35))
        .border_1()
        .border_color(white(0.14))
        .cursor_pointer()
        .relative()
        .child(hover_fill(id, white(0.1), 8.0))
        .child(icon(name, 16.0, white(0.9)))
}

/// Artist hero: big round white play button (pause while `playing`).
fn hero_play(id: &'static str, playing: bool, loading: bool) -> Stateful<gpui::Div> {
    with_tip(div().id(id), tip_for(if playing { "pause" } else { "play" }))
        .flex_none()
        .size(px(48.0))
        .rounded_full()
        .bg(gpui::white())
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .relative()
        .child(hover_fill(id, black(0.1), 24.0))
        // The triangle's weight sits left; nudge it to look centred.
        .child(div().when(!playing && !loading, |d| d.pl(px(2.0))).child(play_glyph(loading, playing, 20.0, 48.0)))
}

/// Artist hero: borderless icon action next to the play button; its hover
/// is a rounded box like the follow button's.
fn hero_icon(id: &'static str, name: &str) -> Stateful<gpui::Div> {
    with_tip(div().id(id), tip_for(name))
        .flex_none()
        .size(px(36.0))
        .rounded(px(8.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .relative()
        .child(hover_fill(id, white(0.1), 8.0))
        .child(icon(name, 18.0, white(0.85)))
}

fn play_pill(id: impl Into<ElementId>, label: SharedString) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .h(px(36.0))
        .px(px(18.0))
        .rounded(px(8.0))
        .bg(gpui::white())
        .cursor_pointer()
        .text_size(theme::text_sm())
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(gpui::black())
        .hover(|s| s.opacity(0.88))
        .child(icon("play", 14.0, gpui::black()))
        .child(label)
}

// ------------------------------------------------------------------ list view

pub fn list_view(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    let Some(l) = app.open_list.clone() else { return div().into_any_element() };
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
                                .child(div().text_size(px(22.0)).line_height(px(28.0)).font_weight(FontWeight::BOLD).text_color(theme::text()).child(title.clone()))
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
                    .child(play_pill("list-play", t("list.play_all")).flex_1().on_click(cx.listener(move |this, _, _, cx| this.play_list(t_play.clone(), 0, cx))))
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
                        Some(badge) => crate::ui::widgets::with_tip(div().id("list-download"), crate::ui::widgets::tip_for("download"))
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
                        None => icon_button("list-download", "download", 16.0, theme::text_dim()),
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

    let right: AnyElement = if tracks.is_empty() {
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
        let n = list.len();
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

    // The cover column stays put; only the songs scroll.
    div()
        .size_full()
        .flex()
        .flex_col()
        .gap(px(16.0))
        .px(px(32.0))
        .pt(px(20.0))
        .when_some(drop_id, |d, id| {
            d.drag_over::<ExternalPaths>(|s, _, _, _| s.bg(theme::hover()))
                .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                    this.import_files_to_playlist(id.clone(), paths.paths().to_vec(), cx)
                }))
        })
        .children(back_button(app, cx).map(|b| div().flex_none().flex().child(b)))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .gap(px(36.0))
                .child(left)
                .child(
                    div()
                        .id("list-scroll")
                        .track_scroll(&app.main_scroll)
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .overflow_y_scroll()
                        .pb(px(20.0))
                        .child(right),
                ),
        )
        .into_any_element()
}

// ------------------------------------------------------------------ artist

pub fn artist_view(app: &mut Glyph, window: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    let Some(name) = app.open_artist.clone() else { return div().into_any_element() };
    let (tracks, loading, info, info_loading, show_all, about_open) = match app.artists.get(&name) {
        Some(p) => (p.tracks.clone(), p.loading, p.info.clone(), p.info_loading, p.show_all, p.about_open),
        None => (Vec::new(), true, None, true, false, false),
    };
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
                                hero_play("artist-play", playing, loading).mr(px(6.0)).on_click(cx.listener(move |this, _, _, cx| {
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

    // Albums, grouped from the songs found.
    let mut albums: Vec<(String, Vec<Track>)> = Vec::new();
    for tr in &tracks {
        if let Some(a) = tr.album.clone().filter(|a| !a.trim().is_empty()) {
            match albums.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(&a)) {
                Some((_, v)) => v.push(tr.clone()),
                None => albums.push((a, vec![tr.clone()])),
            }
        }
    }
    let album_cards: Vec<AnyElement> = albums
        .into_iter()
        .enumerate()
        .map(|(i, (title, list))| {
            let art = cover(app, &list[0], 160.0, 8.0, cx);
            let label = app.album_label(&title, &name, list.len(), cx);
            let l = ListRef::Album { title: title.clone(), artist: name.clone(), tracks: list };
            let lm = l.clone();
            with_list_menu(div().id(("album", i)), lm, cx)
                .flex_none()
                .w(px(176.0))
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
        })
        .collect();

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
            d.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(section_title("disc-3", t("artist.albums")))
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
        });
    div().id("artist-scroll").track_scroll(&app.main_scroll).size_full().overflow_y_scroll().px(px(32.0)).py(px(20.0)).child(body).into_any_element()
}

// ------------------------------------------------------------------ library

pub fn library(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    let head = div()
        .flex()
        .items_center()
        .justify_between()
        .child(heading(t("nav.library")))
        .child(
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
        );
    if !app.library_ok {
        return div()
            .id("lib-scroll")
        .track_scroll(&app.main_scroll)
            .size_full()
            .px(px(32.0))
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
        lists.push((ListRef::User(p.id.clone()), t("list.kind.playlist").to_string()));
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
                .child(div().pt(px(6.0)).truncate().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).text_color(theme::text()).child(f.name.clone()))
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
    div()
        .id("lib-scroll")
        .track_scroll(&app.main_scroll)
        .size_full()
        .overflow_y_scroll()
        .px(px(32.0))
        .py(px(20.0))
        .child(div().flex().flex_col().gap(px(18.0)).child(head).child(div().flex().flex_wrap().gap(px(8.0)).ml(px(-8.0)).children(tiles)).children(artists_section))
        .into_any_element()
}

// ------------------------------------------------------------------ song menu

/// "Add to playlist" row with its hover flyout (`sub`).
fn add_pl_row(open: bool, flip: bool, sub: Vec<AnyElement>, whole_list: bool, cx: &mut Context<Glyph>) -> AnyElement {
    // On a list it copies all of its songs, so it says so.
    let label = if whole_list { t("ctx.add_all_playlist") } else { t("ctx.add_playlist") };
    ctx_item("ctx-add-pl".into(), "list-music", label, white(0.7), false, cx)
                .relative()
                .when(open, |d| d.bg(white(0.07)))
                .child(icon("chevron-right", 14.0, white(0.4)))
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    if let Some(m) = this.ctx_menu.as_mut().filter(|m| *hovered && !m.playlists) {
                        m.playlists = true;
                        cx.notify();
                    }
                }))
                .when(open, |d| {
                    d.child(
                        div()
                            .id("ctx-sub")
                            .track_scroll(&crate::ui::widgets::handle_for("ctx-sub"))
                            .child(crate::ui::widgets::smooth_wheel(&crate::ui::widgets::handle_for("ctx-sub"), false, cx))
                            .absolute()
                            .top(px(-6.0))
                            .when(!flip, |d| d.left(px(236.0)))
                            .when(flip, |d| d.right(px(236.0)))
                            .w(px(228.0))
                            .max_h(px(320.0))
                            .overflow_y_scroll()
                            .p(px(6.0))
                            .rounded(px(10.0))
                            .bg(gpui::hsla(0.0, 0.0, 0.075, 0.98))
                            .border_1()
                            .border_color(white(0.1))
                            .flex()
                            .flex_col()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .children(sub),
                    )
                })
                .into_any_element()
}

/// Destructive row (remove, hide, delete): turns red on hover.
fn danger_item(id: ElementId, name: &str, label: SharedString, cx: &mut Context<Glyph>) -> Stateful<gpui::Div> {
    menu_row(id, name, label, gpui::hsla(0.0, 0.75, 0.68, 0.9), true, true, cx)
}

/// One menu row. `closes_sub`: hovering it closes the playlist flyout.
fn ctx_item(id: ElementId, name: &str, label: SharedString, color: Hsla, closes_sub: bool, cx: &mut Context<Glyph>) -> Stateful<gpui::Div> {
    menu_row(id, name, label, color, closes_sub, false, cx)
}

/// A div takes only one hover style, so normal and red rows share this.
fn menu_row(id: ElementId, name: &str, label: SharedString, color: Hsla, closes_sub: bool, danger: bool, cx: &mut Context<Glyph>) -> Stateful<gpui::Div> {
    // Hover fades in and out; red rows also fade their text to red.
    let key = format!("menu-{id:?}");
    let k = hover_k(&key);
    let text = if danger { mix(white(0.88), gpui::hsla(0.0, 0.85, 0.75, 1.0), k) } else { white(0.88) };
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(10.0))
        .h(px(32.0))
        .px(px(10.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .text_size(px(13.0))
        .text_color(text)
        .relative()
        .child(hover_fill(key, if danger { gpui::hsla(0.0, 0.7, 0.5, 0.18) } else { white(0.07) }, 6.0))
        .when(closes_sub, |d| {
            d.on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if let Some(m) = this.ctx_menu.as_mut().filter(|m| *hovered && m.playlists) {
                    m.playlists = false;
                    cx.notify();
                }
            }))
        })
        .child(icon(name, 15.0, color))
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
    let tr = m.track.clone();
    let sep = || div().h(px(1.0)).mx(px(6.0)).my(px(4.0)).bg(white(0.07));
    let mut body: Vec<AnyElement> = Vec::new();
    // Songs the menu acts on: one song, or a whole list.
    let songs: Vec<Track> = match &m.list {
        Some(l) => app.list_tracks(l),
        None => vec![tr.clone()],
    };
    // "Add to playlist" flyout (opens on hover, to the right or left).
    let mut sub: Vec<AnyElement> = Vec::new();
    if m.playlists {
        let t_new = songs.clone();
        sub.push(
            ctx_item("ctx-new-pl".into(), "plus", t("library.playlist.new"), white(0.7), false, cx)
                .on_click(cx.listener(move |this, _, w, cx| this.open_name_dialog(t_new.clone(), None, w, cx)))
                .into_any_element(),
        );
        if !app.library.playlists.is_empty() {
            sub.push(sep().into_any_element());
        }
        let pls: Vec<(String, String)> = app.library.playlists.iter().map(|p| (p.id.clone(), p.name.clone())).collect();
        let own = match &m.list {
            Some(ListRef::User(id)) => Some(id.clone()),
            _ => None,
        };
        for (i, (id, name)) in pls.into_iter().enumerate().filter(|(_, (id, _))| Some(id) != own.as_ref()) {
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
    let flip = f32::from(m.pos.x) + 244.0 + 236.0 > f32::from(window.viewport_size().width);
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
        let l_open = l.clone();
        body.push(
            ctx_item("lctx-open".into(), "list-music", t("ctx.open"), white(0.7), true, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.begin_ctx_close();
                    this.open_list(l_open.clone(), cx)
                }))
                .into_any_element(),
        );
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
        if has {
            body.push(add_pl_row(m.playlists, flip, sub, true, cx));
        }
        if matches!(l, ListRef::Album { .. }) {
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
        let t1 = tr.clone();
        body.push(
            ctx_item("ctx-play".into(), "play", t("ctx.play"), white(0.7), true, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.begin_ctx_close();
                    this.play_list(vec![t1.clone()], 0, cx)
                }))
                .into_any_element(),
        );
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
        body.push(sep().into_any_element());
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
        body.push(
            add_pl_row(m.playlists, flip, sub, false, cx),
        );
        if let Some((l, i)) = m.from.clone().filter(|(l, _)| matches!(l, ListRef::User(_) | ListRef::Recent)) {
            let label = if l == ListRef::Recent { t("ctx.remove_recent") } else { t("ctx.remove") };
            body.push(
                danger_item("ctx-remove".into(), "circle-minus", label, cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.begin_ctx_close();
                        this.remove_from_list(&l, i, cx)
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

    let menu = div()
        .id("ctx-menu")
        .w(px(244.0))
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

// ------------------------------------------------------------------ name dialog

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
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(8.0))
                .child(button("dialog-cancel", t("dialog.cancel")).on_click(cx.listener(|this, _, _, cx| this.close_name_dialog(cx))))
                .child(
                    primary_button("dialog-ok", if renaming || eq { t("dialog.save") } else { t("dialog.create") })
                        .on_click(cx.listener(|this, _, _, cx| this.submit_name_dialog(cx))),
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
fn edit_dialog(app: &mut Glyph, id: String, cx: &mut Context<Glyph>) -> AnyElement {
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

/// Row pitch of song lists (virtualized; drag slots are computed from it).
const LIST_PITCH: f32 = 56.0;

/// A song of an own playlist being dragged: the row follows the cursor.
#[derive(Clone)]
struct SongDrag {
    list: String,
    from: usize,
    track: Track,
    art: Option<std::path::PathBuf>,
}

impl Render for SongDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let art = match self.art.clone() {
            Some(p) => img(p).flex_none().size(px(40.0)).rounded(px(6.0)).object_fit(ObjectFit::Cover).into_any_element(),
            None => div().flex_none().size(px(40.0)).rounded(px(6.0)).bg(theme::card()).into_any_element(),
        };
        theme::on_panel(|| {
            div()
                .w(px(420.0))
                .h(px(LIST_PITCH))
                .px(px(10.0))
                .flex()
                .items_center()
                .gap(px(14.0))
                .rounded(px(8.0))
                .bg(gpui::hsla(0.0, 0.0, 0.16, 1.0))
                .border_1()
                .border_color(white(0.10))
                .font_family(theme::FONT)
                .text_color(theme::text())
                .cursor_grabbing()
                .child(icon("grip-vertical", 14.0, theme::text()))
                .child(art)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(song_title(self.track.title.clone(), self.track.explicit).text_size(theme::text_md()))
                        .child(div().truncate().text_size(theme::text_sm()).text_color(theme::text_dim()).child(self.track.artist.clone())),
                )
        })
    }
}
