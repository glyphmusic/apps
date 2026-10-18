//! Profiles: picture, banner and a page other signed-in users open by handle
//! ("@name" in the search). Settings -> Profile picks the pictures and what
//! the profile shows (private by default, playlists one by one, recent songs
//! on request). The server keeps it all with the account (`/v1/profile`).

use crate::app::{Glyph, ListRef, SettingsPage, View};
use crate::guard::guard;
use crate::i18n::{t, tf};
use crate::theme;
use crate::ui::settings::{row, rows, section_label, switch, white};
use crate::ui::widgets::*;
use glyph_protocol::{ProfilePatch, ProfileSettings, PublicProfile, Track};
use gpui::{div, img, prelude::*, px, AnyElement, Context, Div, FontWeight, ObjectFit, Window};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

/// Songs sent for "recently played" on the profile.
const RECENT_SHOWN: usize = 20;
/// Quiet time after a song starts before the list is sent.
const RECENT_DEBOUNCE: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct ProfileUi {
    /// The open profile (`Glyph::open_profile`).
    pub page: Option<PublicProfile>,
    pub loading: bool,
    pub error: Option<String>,
    /// Pictures and covers by sha256: local file (`None` = loading or missing).
    pub images: HashMap<String, Option<PathBuf>>,
    /// Songs of profile playlists the server sent without a preview, for
    /// their automatic cover (by `handle/id`; `None` = loading or failed).
    pub previews: HashMap<String, Option<Vec<Track>>>,
    /// Own settings (Settings -> Profile), `None` until loaded.
    pub settings: Option<ProfileSettings>,
    /// An image is being prepared / uploaded.
    pub busy: bool,
    pub notice: Option<(String, bool)>,
    recent_gen: u64,
    /// Keys of the songs last sent, so unchanged lists are not sent again.
    recent_sent: Vec<String>,
}

/// `@name` typed in the search: the handle, if it can be one.
pub fn handle_query(q: &str) -> Option<String> {
    let h = q.trim().strip_prefix('@')?.to_ascii_lowercase();
    ((3..=20).contains(&h.len()) && h.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')).then_some(h)
}

fn err_text(e: &anyhow::Error) -> String {
    if glyph_core::account::is_signed_out(e) {
        return t("account.signed_out").to_string();
    }
    match glyph_core::server_client::server_error(e).map(|(s, c)| (s, c.to_string())) {
        Some((404, _)) => t("profile.not_found").to_string(),
        Some((413, _)) => t("profile.image.too_big").to_string(),
        Some((_, c)) if c == "rate_limited" => t("account.err.slow").to_string(),
        _ => t("account.err.generic").to_string(),
    }
}

impl Glyph {
    /// Opens the profile of `handle` (with or without `@`).
    pub fn open_profile(&mut self, handle: &str, cx: &mut Context<Self>) {
        let h = handle.trim().trim_start_matches('@').to_ascii_lowercase();
        if h.is_empty() || (self.view == View::Profile && self.open_profile.as_deref() == Some(h.as_str())) {
            return;
        }
        self.push_route();
        self.view = View::Profile;
        self.main_scroll.set_offset(gpui::point(px(0.0), px(0.0)));
        self.open_profile = Some(h.clone());
        self.open_list = None;
        self.open_artist = None;
        self.ctx_menu = None;
        self.load_profile(h, cx);
        cx.notify();
    }

    pub fn load_profile(&mut self, handle: String, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        if self.profile.page.as_ref().is_some_and(|p| p.handle != handle) {
            self.profile.page = None;
        }
        self.profile.loading = true;
        self.profile.error = None;
        // Playlists may have changed since: fetch their songs again.
        self.profile.previews.clear();
        cx.spawn(async move |this, cx| {
            let r = b.profile(&handle).await;
            this.update(cx, |s, cx| {
                if s.open_profile.as_deref() != Some(handle.as_str()) {
                    return;
                }
                s.profile.loading = false;
                match r {
                    Ok(p) => s.profile.page = Some(p),
                    Err(e) => {
                        s.profile.page = None;
                        s.profile.error = Some(err_text(&e));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Local file of a profile picture / cover; fetched once in the background.
    pub fn profile_image(&mut self, handle: &str, sha: &str, cx: &mut Context<Self>) -> Option<PathBuf> {
        if let Some(p) = self.profile.images.get(sha) {
            return p.clone();
        }
        let Some(b) = self.backend.clone() else { return None };
        self.profile.images.insert(sha.to_string(), None);
        let (h, sha) = (handle.to_string(), sha.to_string());
        cx.spawn(async move |this, cx| {
            let r = b.profile_image(&h, &sha).await;
            if let Err(e) = &r {
                tracing::info!("profile image: {e:#}");
            }
            this.update(cx, |s, cx| {
                s.profile.images.insert(sha, r.ok());
                cx.notify();
            })
            .ok();
        })
        .detach();
        None
    }

    /// First songs of a profile playlist for its automatic cover, when the
    /// profile came without them (older servers); fetched once.
    pub fn profile_preview(&mut self, handle: &str, id: &str, cx: &mut Context<Self>) -> Option<Vec<Track>> {
        let key = format!("{handle}/{id}");
        if let Some(p) = self.profile.previews.get(&key) {
            return p.clone();
        }
        let Some(b) = self.backend.clone() else { return None };
        self.profile.previews.insert(key.clone(), None);
        let (h, id) = (handle.to_string(), id.to_string());
        cx.spawn(async move |this, cx| {
            let r = b.profile_playlist(&h, &id).await;
            if let Err(e) = &r {
                tracing::info!("profile playlist preview: {e:#}");
            }
            this.update(cx, |s, cx| {
                s.profile.previews.insert(key, r.ok().map(|p| p.tracks.into_iter().take(12).collect()));
                cx.notify();
            })
            .ok();
        })
        .detach();
        None
    }

    /// Opens a playlist shown on a profile (its songs come from the server).
    pub fn open_profile_playlist(&mut self, handle: String, id: String, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        cx.spawn(async move |this, cx| {
            let r = b.profile_playlist(&handle, &id).await;
            this.update(cx, |s, cx| {
                match r {
                    Ok(p) => s.open_list(ListRef::Album { title: p.name, artist: format!("@{}", p.handle), tracks: p.tracks }, cx),
                    Err(e) => s.toast_info(err_text(&e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ------------------------------------------------------------ settings

    /// Settings -> Profile opened: load the own settings.
    pub fn profile_page_opened(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        if self.signed_in().is_none() {
            return;
        }
        self.profile.notice = None;
        cx.spawn(async move |this, cx| {
            let r = b.profile_settings().await;
            this.update(cx, |s, cx| {
                match r {
                    Ok(p) => s.profile.settings = Some(p),
                    Err(e) => s.profile.notice = Some((err_text(&e), false)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn profile_update(&mut self, patch: ProfilePatch, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        // Shown at once; the server's answer replaces it.
        if let Some(s) = self.profile.settings.as_mut() {
            if let Some(v) = patch.public {
                s.public = v;
            }
            if let Some(v) = patch.show_recent {
                s.show_recent = v;
            }
            if let Some(v) = patch.playlists.clone() {
                s.playlists = v;
            }
        }
        let recent_on = patch.show_recent == Some(true);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = b.profile_update(patch).await;
            this.update(cx, |s, cx| {
                match r {
                    Ok(p) => {
                        s.profile.settings = Some(p);
                        s.profile.notice = None;
                        if recent_on {
                            s.profile.recent_sent.clear();
                            s.profile_recent_changed(cx);
                        }
                    }
                    Err(e) => {
                        s.profile.notice = Some((err_text(&e), false));
                        s.profile_page_opened(cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Show or hide one playlist on the profile.
    pub fn profile_toggle_playlist(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(s) = self.profile.settings.as_ref() else { return };
        let mut ids = s.playlists.clone();
        match ids.iter().position(|x| *x == id) {
            Some(i) => {
                ids.remove(i);
            }
            None => ids.push(id),
        }
        // Playlists deleted meanwhile drop out.
        ids.retain(|i| self.library.playlist(i).is_some());
        self.profile_update(ProfilePatch { playlists: Some(ids), ..Default::default() }, cx);
    }

    /// Lets the user pick a picture (`banner`: the wide banner) and uploads it.
    pub fn profile_pick_image(&mut self, banner: bool, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t(if banner { "profile.banner.pick" } else { "profile.avatar.pick" }).into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(src) = paths.into_iter().next() else { return };
            this.update(cx, |s, cx| {
                s.profile.busy = true;
                cx.notify();
            })
            .ok();
            let bytes = cx
                .background_executor()
                .spawn(async move {
                    glyph_core::profile_images::prepare(&src,banner).map_err(|e| e.to_string())
                })
                .await;
            let r = match bytes {
                Ok(bytes) => b.profile_set_image(banner, Some(bytes)).await.map_err(|e| err_text(&e)),
                Err(e) => Err(tf("toast.cover_error", &[("e", &e)]).to_string()),
            };
            this.update(cx, |s, cx| {
                s.profile.busy = false;
                match r {
                    Ok(p) => {
                        s.profile.settings = Some(p);
                        s.profile.notice = None;
                        s.profile_refresh_open(cx);
                    }
                    Err(e) => s.profile.notice = Some((e, false)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn profile_remove_image(&mut self, banner: bool, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        cx.spawn(async move |this, cx| {
            let r = b.profile_set_image(banner, None).await;
            this.update(cx, |s, cx| {
                match r {
                    Ok(p) => {
                        s.profile.settings = Some(p);
                        s.profile_refresh_open(cx);
                    }
                    Err(e) => s.profile.notice = Some((err_text(&e), false)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The own profile is open behind settings: show the change there too.
    fn profile_refresh_open(&mut self, cx: &mut Context<Self>) {
        if let (View::Profile, Some(h)) = (self.view, self.open_profile.clone()) {
            if self.profile.page.as_ref().is_some_and(|p| p.own) {
                self.load_profile(h, cx);
            }
        }
    }

    /// Opens the own profile (from settings).
    pub fn profile_view_own(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(acc) = self.signed_in() else { return };
        self.close_settings(window, cx);
        self.open_profile(&acc.account.handle, cx);
    }

    /// A song started: send the recent songs shortly after, when the profile shows them.
    pub fn profile_recent_changed(&mut self, cx: &mut Context<Self>) {
        if self.signed_in().is_none() {
            return;
        }
        self.profile.recent_gen += 1;
        let gen = self.profile.recent_gen;
        let Some(b) = self.backend.clone() else { return };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RECENT_DEBOUNCE).await;
            // Settings not known yet: ask once.
            let known = this.update(cx, |s, _| s.profile.settings.as_ref().map(|p| p.show_recent)).ok().flatten();
            let on = match known {
                Some(on) => on,
                None => match b.profile_settings().await {
                    Ok(p) => {
                        let on = p.show_recent;
                        this.update(cx, |s, _| s.profile.settings = Some(p)).ok();
                        on
                    }
                    Err(_) => return,
                },
            };
            let Some(tracks) = this
                .update(cx, |s, _| {
                    if !on || s.profile.recent_gen != gen {
                        return None;
                    }
                    let tracks: Vec<Track> = s.library.recent.iter().take(RECENT_SHOWN).cloned().collect();
                    let keys: Vec<String> = tracks.iter().map(crate::app::track_key).collect();
                    if keys == s.profile.recent_sent {
                        return None;
                    }
                    s.profile.recent_sent = keys;
                    Some(tracks)
                })
                .ok()
                .flatten()
            else {
                return;
            };
            if let Err(e) = b.profile_push_recent(tracks).await {
                tracing::info!("profile recent: {e:#}");
                this.update(cx, |s, _| s.profile.recent_sent.clear()).ok();
            }
        })
        .detach();
    }
}

// ------------------------------------------------------------------ search

/// "Open profile @name" on top of the search results.
pub fn search_hit(app: &mut Glyph, query: &str, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    let h = handle_query(query)?;
    app.signed_in()?;
    let label = tf("profile.search.open", &[("handle", &h)]);
    Some(
        div()
            .id("search-profile")
            .flex()
            .items_center()
            .gap(px(14.0))
            .px(px(10.0))
            .py(px(6.0))
            .rounded(px(8.0))
            .cursor_pointer()
            .relative()
            .child(hover_fill("search-profile", theme::card(), 8.0))
            .on_click(cx.listener(move |this, _, _, cx| this.open_profile(&h, cx)))
            .child(
                div()
                    .flex_none()
                    .size(px(48.0))
                    .rounded_full()
                    .bg(white(0.08))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("user-round", 20.0, white(0.75))),
            )
            .child(div().flex_1().min_w_0().truncate().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).text_color(theme::text()).child(label))
            .child(kind_chip(t("profile.kind")))
            .into_any_element(),
    )
}

// ------------------------------------------------------------------ page

fn avatar(path: Option<PathBuf>, size: f32) -> Div {
    let base = div()
        .flex_none()
        .size(px(size))
        .rounded_full()
        .overflow_hidden()
        .bg(white(0.10))
        .border_2()
        .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.6))
        .flex()
        .items_center()
        .justify_center();
    match path {
        Some(p) => base.child(img(p).size(px(size)).rounded_full().object_fit(ObjectFit::Cover)),
        None => base.child(icon("user-round", size * 0.42, white(0.6))),
    }
}

pub fn view(app: &mut Glyph, window: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    let Some(handle) = app.open_profile.clone() else { return div().into_any_element() };
    let scroll = app.main_scroll.clone();
    let wrap = move |el: AnyElement| div().id("profile-scroll").track_scroll(&scroll).size_full().overflow_y_scroll().px(px(32.0)).py(px(20.0)).child(el).into_any_element();
    let p = app.profile.page.clone().filter(|p| p.handle == handle);
    let Some(p) = p else {
        let body = if app.signed_in().is_none() {
            ascii_state(ART_EMPTY, t("profile.signed_out"), t("profile.signed_out.sub"))
        } else if let Some(e) = app.profile.error.clone() {
            ascii_state(ART_OFFLINE, e, format!("@{handle}"))
        } else {
            div().flex().flex_col().gap(px(16.0)).child(skeleton("profile-banner", 1680.0, 220.0, 14.0)).child(skeleton_rows("profile", 5)).into_any_element()
        };
        return wrap(div().flex().flex_col().gap(px(16.0)).children(crate::ui::lists::back_button(app, cx)).child(body).into_any_element());
    };

    // The banner with its fade baked in (like the artist hero), so it melts
    // into the page instead of ending on an edge.
    const BANNER_H: f32 = 260.0;
    for im in app.stale_heroes.drain(..) {
        let _ = window.drop_image(im);
    }
    let min_w = f32::from(window.viewport_size().width) * 300.0 / BANNER_H;
    let banner = p.banner.clone().and_then(|s| app.profile_image(&p.handle, &s, cx)).and_then(|path| app.hero_image(&path, min_w, 300, true, cx));
    let pic = p.avatar.clone().and_then(|s| app.profile_image(&p.handle, &s, cx));
    let hero = div()
        .relative()
        .mx(px(-24.0))
        .mt(px(-12.0))
        .h(px(BANNER_H))
        .rounded_t(px(14.0))
        .overflow_hidden()
        // Like the artist hero, it melts into the page at the bottom instead
        // of ending on an edge: colour on top, fully transparent below.
        .when(banner.is_none(), |d| {
            d.child(div().absolute().inset_0().rounded_t(px(14.0)).bg(gpui::linear_gradient(
                0.0,
                gpui::linear_color_stop(gpui::hsla(300.0 / 360.0, 0.40, 0.16, 0.0), 0.0),
                gpui::linear_color_stop(theme::accent_tile(0.22, 0.45), 1.0),
            )))
        })
        .when_some(banner, |d, b| {
            let k = reveal_k(&format!("profile:banner:{}", p.handle), 0.5);
            d.child(img(b).absolute().inset_0().size_full().rounded_t(px(14.0)).object_fit(ObjectFit::Cover).opacity(k))
        })
        .children(crate::ui::lists::back_button(app, cx).map(|b| div().absolute().top(px(20.0)).left(px(32.0)).child(b)));

    let own = p.own;
    let name_row = div()
        .flex()
        .items_end()
        .gap(px(20.0))
        .mt(px(-56.0 - 22.0))
        .px(px(8.0))
        .child(avatar(pic, 112.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .pb(px(56.0 + 14.0))
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(mono().text_size(px(11.0)).text_color(white(0.55)).child(format!("[{}]", t(if p.private { "profile.badge.private" } else { "profile.badge" }))))
                .child(div().text_size(px(36.0)).line_height(px(40.0)).font_weight(FontWeight::BOLD).text_color(gpui::white()).truncate().child(format!("@{}", p.handle))),
        )
        .when(own, |d| {
            d.child(div().pb(px(56.0 + 18.0)).child(button("profile-edit", t("profile.edit")).on_click(cx.listener(|this, _, _, cx| {
                this.open_settings(Some(SettingsPage::Profile), cx);
                this.profile_page_opened(cx);
            }))))
        });

    let mut body = div().flex().flex_col().gap(px(22.0)).child(hero).child(name_row);
    if own && p.private {
        body = body.child(div().text_size(theme::text_sm()).text_color(white(0.55)).child(t("profile.own_private")));
    }
    if p.private && !own {
        body = body.child(ascii_state(ART_EMPTY, t("profile.private"), t("profile.private.sub")));
        return wrap(body.into_any_element());
    }

    if !p.playlists.is_empty() {
        let cards: Vec<AnyElement> = p
            .playlists
            .iter()
            .enumerate()
            .map(|(i, pl)| {
                // Own picture first; else the automatic cover from the songs:
                // the local playlist on the own profile, the preview the
                // server sent, or (servers without previews) its songs.
                let local = own && app.library.playlist(&pl.id).is_some();
                let songs = if local || pl.cover.is_some() || !pl.preview.is_empty() || pl.songs == 0 {
                    pl.preview.clone()
                } else {
                    app.profile_preview(&p.handle, &pl.id, cx).unwrap_or_default()
                };
                let art = match pl.cover.clone().and_then(|s| app.profile_image(&p.handle, &s, cx)) {
                    Some(path) => img(path).size(px(160.0)).rounded(px(8.0)).object_fit(ObjectFit::Cover).into_any_element(),
                    None if local => crate::ui::lists::list_cover(app, &ListRef::User(pl.id.clone()), 160.0, cx),
                    None if !songs.is_empty() => crate::ui::lists::list_cover(
                        app,
                        &ListRef::Album { title: pl.name.clone(), artist: format!("@{}", p.handle), tracks: songs },
                        160.0,
                        cx,
                    ),
                    None => div()
                        .size(px(160.0))
                        .rounded(px(8.0))
                        .bg(white(0.06))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon("music-2", 32.0, white(0.4)))
                        .into_any_element(),
                };
                let (h, id) = (p.handle.clone(), pl.id.clone());
                div()
                    .id(("profile-pl", i))
                    .flex_none()
                    .w(px(176.0))
                    .p(px(8.0))
                    .rounded(px(10.0))
                    .relative()
                    .child(hover_fill(format!("profile-pl-{i}"), white(0.05), 10.0))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.open_profile_playlist(h.clone(), id.clone(), cx)))
                    .child(art)
                    .child(div().pt(px(4.0)).truncate().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).text_color(theme::text()).child(pl.name.clone()))
                    .child(mono().text_size(px(11.0)).text_color(theme::text_faint()).child(tf("common.n_songs", &[("n", &pl.songs)]).to_uppercase()))
                    .into_any_element()
            })
            .collect();
        body = body.child(div().flex().flex_col().gap(px(12.0)).child(section_title("library", t("profile.playlists"))).child(div().flex().flex_wrap().gap(px(4.0)).ml(px(-8.0)).children(cards)));
    }
    if !p.recent.is_empty() {
        let list = Rc::new(p.recent.iter().take(10).cloned().collect::<Vec<_>>());
        let song_rows: Vec<_> = (0..list.len()).map(|i| crate::ui::lists::song_row(app, list.clone(), i, "profile-row", None, true, cx)).collect();
        body = body.child(div().flex().flex_col().gap(px(8.0)).child(section_title("music-2", t("profile.recent"))).child(div().flex().flex_col().children(song_rows)));
    }
    if p.playlists.is_empty() && p.recent.is_empty() {
        body = body.child(ascii_state(ART_EMPTY, t("profile.empty"), if own { t("profile.empty.own") } else { t("profile.empty.sub") }));
    }
    wrap(body.into_any_element())
}

// ------------------------------------------------------------------ settings

pub fn settings_page(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let col = div().flex().flex_col();
    let Some(acc) = app.signed_in() else {
        return col.child(div().py(px(14.0)).text_size(px(13.0)).text_color(white(0.6)).child(t("profile.signed_out.sub")));
    };
    let notice = app.profile.notice.clone().map(|(m, ok)| div().pt(px(8.0)).text_size(px(12.5)).text_color(if ok { theme::ok() } else { theme::warn() }).child(m));
    let Some(s) = app.profile.settings.clone() else {
        return if notice.is_some() {
            col.children(notice).child(button("profile-settings-retry", t("common.retry"))
                .on_click(cx.listener(|this, _, _, cx| this.profile_page_opened(cx))))
        } else { col.child(skeleton_rows("profile-settings", 3)) };
    };
    let handle = acc.account.handle.clone();
    if app.settings_sub == 1 {
        return privacy(app, &s, col, cx).children(notice);
    }

    let busy = app.profile.busy;
    let pic = s.avatar.clone().and_then(|sha| app.profile_image(&handle, &sha, cx));
    let banner = s.banner.clone().and_then(|sha| app.profile_image(&handle, &sha, cx));
    let image_buttons = |id: &'static str, banner: bool, set: bool, cx: &mut Context<Glyph>| {
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(button((id, 0usize), t(if busy { "profile.image.uploading" } else if set { "profile.image.change" } else { "profile.image.pick" })).on_click(cx.listener(move |this, _, _, cx| {
                if !this.profile.busy {
                    this.profile_pick_image(banner, cx)
                }
            })))
            .when(set, |d| d.child(button((id, 1usize), t("profile.image.remove")).on_click(cx.listener(move |this, _, _, cx| this.profile_remove_image(banner, cx)))))
    };
    let banner_preview = div()
        .w(px(120.0))
        .h(px(40.0))
        .rounded(px(6.0))
        .overflow_hidden()
        .bg(white(0.08))
        .when_some(banner, |d, b| d.child(img(b).size_full().object_fit(ObjectFit::Cover)));
    col.child(rows(vec![
        row(
            t("profile.avatar"),
            Some(t("profile.avatar.desc")),
            div().flex().items_center().gap(px(12.0)).child(avatar(pic, 40.0)).child(image_buttons("profile-avatar", false, s.avatar.is_some(), cx)),
        ),
        row(
            t("profile.banner"),
            Some(t("profile.banner.desc")),
            div().flex().items_center().gap(px(12.0)).child(banner_preview).child(image_buttons("profile-banner", true, s.banner.is_some(), cx)),
        ),
        row(
            t("profile.view"),
            Some(tf("profile.view.desc", &[("handle", &handle)])),
            button("profile-view", t("profile.view.button")).on_click(cx.listener(|this, _, w, cx| this.profile_view_own(w, cx))),
        ),
    ]))
    .children(notice)
}

fn privacy(app: &mut Glyph, s: &ProfileSettings, col: Div, cx: &mut Context<Glyph>) -> Div {
    let (public, recent) = (s.public, s.show_recent);
    let top = rows(vec![
        row(
            t("profile.public"),
            Some(t("profile.public.desc")),
            switch("profile-public", public).on_click(cx.listener(move |this, _, _, cx| this.profile_update(ProfilePatch { public: Some(!public), ..Default::default() }, cx))),
        ),
        row(
            t("profile.show_recent"),
            Some(t("profile.show_recent.desc")),
            switch("profile-recent", recent).on_click(cx.listener(move |this, _, _, cx| this.profile_update(ProfilePatch { show_recent: Some(!recent), ..Default::default() }, cx))),
        ),
    ]);
    let playlists: Vec<(String, String, usize)> = guard(|| app.library.playlists.iter().map(|p| (p.id.clone(), p.name.clone(), p.tracks.len())).collect()).unwrap_or_default();
    let list: AnyElement = if playlists.is_empty() {
        div().py(px(14.0)).text_size(px(13.0)).text_color(white(0.55)).child(t("profile.playlists.none")).into_any_element()
    } else {
        rows(
            playlists
                .into_iter()
                .enumerate()
                .map(|(i, (id, name, n))| {
                    let on = s.playlists.contains(&id);
                    row(
                        name,
                        Some(tf("common.n_songs", &[("n", &n)])),
                        switch(("profile-pl", i), on).on_click(cx.listener(move |this, _, _, cx| this.profile_toggle_playlist(id.clone(), cx))),
                    )
                })
                .collect(),
        )
        .into_any_element()
    };
    col.child(top)
        .child(section_label(t("profile.playlists.section")))
        .child(div().pt(px(2.0)).text_size(px(12.5)).text_color(white(0.5)).child(t(if public { "profile.playlists.desc" } else { "profile.playlists.desc_private" })))
        .child(list)
}

