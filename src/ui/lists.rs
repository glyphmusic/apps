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
use glyph_core::RecentItem;
use glyph_protocol::Track;
use gpui::{
    anchored, deferred, div, img, linear_color_stop, linear_gradient, prelude::*, px, AnyElement, Context, ElementId, ExternalPaths,
    FontWeight, Hsla, MouseButton, MouseDownEvent, ObjectFit, SharedString, Stateful, Window,
};
use std::rc::Rc;

mod rows;
mod list_view;
mod artist;
mod library;
mod ctx_menu;
mod dialogs;
pub use rows::*;
pub use list_view::*;
pub use artist::*;
pub use library::*;
pub use ctx_menu::*;
pub use dialogs::*;

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

/// An artist's name with the verified badge right of it when they are verified.
pub fn artist_name_badged(app: &mut Glyph, name: &str, text: impl IntoElement, badge: f32, cx: &mut Context<Glyph>) -> gpui::Div {
    let verified = app.artist_verified(name, cx);
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .min_w_0()
        .child(div().min_w_0().truncate().child(text))
        .when(verified, |d| d.child(verified_badge(badge)))
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
    if crate::ui::compact::TOUCH {
        return None;
    }
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

/// Phones: a long press runs `f` (what a right click does on the desktop).
/// GPUI's divs have no long-press listener, so a canvas over `el` registers
/// one for its bounds.
pub fn on_long_press(el: Stateful<gpui::Div>, f: impl Fn(&gpui::LongPressEvent, &mut Window, &mut gpui::App) + 'static) -> Stateful<gpui::Div> {
    if !crate::ui::compact::TOUCH {
        return el;
    }
    el.child(
        gpui::canvas(
            |b, w, _| w.insert_hitbox(b, gpui::HitboxBehavior::Normal),
            move |_, hitbox, w, _| {
                w.on_mouse_event(move |e: &gpui::LongPressEvent, phase, w, cx| {
                    if phase.bubble() && e.phase == gpui::TouchPhase::Started && hitbox.is_hovered(w) {
                        cx.stop_propagation();
                        w.prevent_default();
                        f(e, w, cx);
                    }
                });
            },
        )
        .absolute()
        .inset_0(),
    )
}

/// Right-click on an artist (name, tile, row) opens the artist menu.
pub fn with_artist_menu(el: Stateful<gpui::Div>, name: String, cx: &mut Context<Glyph>) -> Stateful<gpui::Div> {
    let nm = name.clone();
    let el = on_long_press(el, cx.listener(move |this, e: &gpui::LongPressEvent, _, cx| this.open_artist_ctx(&crate::app::primary_artist(&nm), e.position, cx)));
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
    let lp = l.clone();
    let el = on_long_press(el, cx.listener(move |this, e: &gpui::LongPressEvent, _, cx| this.open_list_ctx(lp.clone(), e.position, cx)));
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
    let (tp, fp) = (tr.clone(), from.clone());
    let el = on_long_press(el, cx.listener(move |this, e: &gpui::LongPressEvent, _, cx| this.open_ctx(tp.clone(), e.position, fp.clone(), cx)));
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
                        .child(song_title(self.track.title.clone(), crate::ui::widgets::rating(&self.track)).text_size(theme::text_md()))
                        .child(div().truncate().text_size(theme::text_sm()).text_color(theme::text_dim()).child(self.track.artist.clone())),
                )
        })
    }
}
