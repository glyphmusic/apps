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
use glyph_core::profile_images::{crop_rect, Crop};
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
    /// Handles looked up for the search: `None` = still checking,
    /// `Some(None)` = no such profile, `Some(Some(avatar sha))` = exists.
    pub lookups: HashMap<String, Option<Option<Option<String>>>>,
    /// Playlist row being dragged in Settings -> Privacy: (from, drop slot).
    pub pl_drop: Option<(usize, usize)>,
    /// Picked picture waiting in the crop dialog.
    pub crop: Option<CropDialog>,
    recent_gen: u64,
    /// Keys of the songs last sent, so unchanged lists are not sent again.
    recent_sent: Vec<String>,
}

/// Crop dialog: drag to move, slider / wheel to zoom.
pub struct CropDialog {
    pub path: PathBuf,
    pub banner: bool,
    /// Image size in pixels.
    pub size: (u32, u32),
    pub crop: Crop,
    /// Drag start: mouse position and crop center then.
    drag: Option<((f32, f32), (f32, f32))>,
}

/// Longest zoom the slider offers.
const CROP_MAX_ZOOM: f32 = 4.0;
/// Crop window in the dialog (picture: square, banner: 3:1).
const CROP_AVATAR: f32 = 280.0;
const CROP_BANNER: (f32, f32) = (432.0, 144.0);
/// Area around the crop window.
const CROP_STAGE: (f32, f32) = (480.0, 320.0);

impl CropDialog {
    fn window(&self) -> (f32, f32) {
        if self.banner { CROP_BANNER } else { (CROP_AVATAR, CROP_AVATAR) }
    }

    /// Screen pixels per image pixel.
    fn scale(&self) -> f32 {
        let (_, _, cw, _) = crop_rect(self.size.0, self.size.1, self.banner, self.crop);
        self.window().0 / cw.max(1) as f32
    }

    /// Keeps the crop region inside the image.
    fn clamp(&mut self) {
        self.crop.zoom = self.crop.zoom.clamp(1.0, CROP_MAX_ZOOM);
        let (w, h) = (self.size.0.max(1) as f32, self.size.1.max(1) as f32);
        let (_, _, cw, ch) = crop_rect(self.size.0, self.size.1, self.banner, self.crop);
        let (hx, hy) = (cw as f32 / w / 2.0, ch as f32 / h / 2.0);
        self.crop.cx = self.crop.cx.clamp(hx, 1.0 - hx);
        self.crop.cy = self.crop.cy.clamp(hy, 1.0 - hy);
    }
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

    /// Checks once whether `handle` has a profile (for the search hit).
    fn lookup_profile(&mut self, handle: &str, cx: &mut Context<Self>) -> Option<Option<Option<String>>> {
        if let Some(v) = self.profile.lookups.get(handle) {
            return v.clone();
        }
        let b = self.backend.clone()?;
        self.profile.lookups.insert(handle.to_string(), None);
        let h = handle.to_string();
        cx.spawn(async move |this, cx| {
            let r = b.profile(&h).await.ok().map(|p| p.avatar);
            this.update(cx, |s, cx| {
                s.profile.lookups.insert(h, Some(r));
                cx.notify();
            })
            .ok();
        })
        .detach();
        None
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
    /// Moves a playlist row from `from` to `to` (in the shown `order`) and
    /// saves the new profile order.
    pub fn profile_reorder_playlist(&mut self, order: &[String], from: usize, to: usize, cx: &mut Context<Self>) {
        let Some(s) = self.profile.settings.as_ref() else { return };
        if from == to || from >= order.len() || to >= order.len() {
            return;
        }
        let mut order = order.to_vec();
        let id = order.remove(from);
        order.insert(to, id);
        let ids: Vec<String> = order.into_iter().filter(|i| s.playlists.contains(i)).collect();
        if ids != s.playlists {
            self.profile_update(ProfilePatch { playlists: Some(ids), ..Default::default() }, cx);
        }
    }

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

    /// Lets the user pick a picture (`banner`: the wide banner) and opens
    /// the crop dialog for it.
    pub fn profile_pick_image(&mut self, banner: bool, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t(if banner { "profile.banner.pick" } else { "profile.avatar.pick" }).into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(src) = paths.into_iter().next() else { return };
            let p = src.clone();
            let size = cx
                .background_executor()
                .spawn(async move { glyph_core::profile_images::dimensions(&p).map_err(|e| e.to_string()) })
                .await;
            this.update(cx, |s, cx| {
                match size {
                    Ok(size) if size.0 > 0 && size.1 > 0 => {
                        s.profile.crop = Some(CropDialog { path: src, banner, size, crop: Crop::default(), drag: None });
                    }
                    Ok(_) => s.profile.notice = Some((tf("toast.cover_error", &[("e", &"empty image")]).to_string(), false)),
                    Err(e) => s.profile.notice = Some((tf("toast.cover_error", &[("e", &e)]).to_string(), false)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn profile_crop_cancel(&mut self, cx: &mut Context<Self>) {
        self.profile.crop = None;
        cx.notify();
    }

    fn profile_crop_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        if let Some(c) = self.profile.crop.as_mut() {
            c.crop.zoom = zoom;
            c.clamp();
            cx.notify();
        }
    }

    /// Crops, compresses and uploads the picture from the crop dialog.
    pub fn profile_crop_apply(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let Some(c) = self.profile.crop.take() else { return };
        self.profile.busy = true;
        cx.notify();
        let (src, banner, crop) = (c.path, c.banner, c.crop);
        cx.spawn(async move |this, cx| {
            let bytes = cx
                .background_executor()
                .spawn(async move { glyph_core::profile_images::prepare_crop(&src, banner, crop).map_err(|e| e.to_string()) })
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
        self.profile.lookups.clear();
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
                    let tracks: Vec<Track> = s.library.recent_tracks().take(RECENT_SHOWN).cloned().collect();
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
    // Only profiles that exist; nothing while checking.
    let found = app.lookup_profile(&h, cx)??;
    let pic = found.and_then(|sha| app.profile_image(&h, &sha, cx));
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
            .child(avatar(pic, 48.0).border_0())
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
            col.children(notice).child(
                div().flex().pt(px(10.0)).child(
                    button("profile-settings-retry", t("common.retry"))
                        .h(px(30.0))
                        .px(px(12.0))
                        .on_click(cx.listener(|this, _, _, cx| this.profile_page_opened(cx))),
                ),
            )
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
            .when(set, |d| d.child(with_tip(
                    div()
                        .id((id, 1usize))
                        .flex_none()
                        .size(px(32.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(8.0))
                        .bg(theme::card())
                        .border_1()
                        .border_color(theme::border())
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::card_hover()))
                        .active(|s| s.bg(theme::active()))
                        .child(icon("trash-2", 15.0, theme::danger())),
                    Some(t("profile.image.remove")),
                ).on_click(cx.listener(move |this, _, _, cx| this.profile_remove_image(banner, cx)))))
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
    // Playlists on the profile first, in their profile order; the rest below.
    let mut playlists: Vec<(String, String, usize)> = guard(|| app.library.playlists.iter().map(|p| (p.id.clone(), p.name.clone(), p.tracks.len())).collect()).unwrap_or_default();
    playlists.sort_by_key(|(id, _, _)| s.playlists.iter().position(|x| x == id).unwrap_or(usize::MAX));
    let order: Rc<Vec<String>> = Rc::new(playlists.iter().map(|p| p.0.clone()).collect());
    let width = px(crate::ui::settings::PANEL_W - crate::ui::settings::NAV_W - 64.0);
    let list: AnyElement = if playlists.is_empty() {
        div().py(px(14.0)).text_size(px(13.0)).text_color(white(0.55)).child(t("profile.playlists.none")).into_any_element()
    } else {
        // Same drag as the queue: the picked row leaves the list and a gap
        // glides open where it would land.
        let n = playlists.len();
        if !cx.has_active_drag() {
            app.profile.pl_drop = None;
        }
        let drop = app.profile.pl_drop;
        let slots = if drop.is_some() { n.saturating_sub(1) } else { n };
        let gaps = crate::ui::views::drag_gaps("profile-pl", PL_ROW_H, slots, drop.map(|(_, at)| at));
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut slot = 0;
        for (i, (id, name, count)) in playlists.into_iter().enumerate() {
            if drop.is_some_and(|(from, _)| from == i) {
                continue;
            }
            let gap = gaps.get(slot).copied().unwrap_or(0.0);
            if gap > 0.5 {
                rows.push(div().flex_none().h(px(gap)).into_any_element());
            }
            slot += 1;
            let on = s.playlists.contains(&id);
            let art = crate::ui::lists::list_cover(app, &ListRef::User(id.clone()), 40.0, cx);
            // Drag image: own picture, else the first song's cover.
            let first = app.library.playlist(&id).and_then(|p| p.tracks.first().cloned());
            let cover_path = app.library.playlist(&id).and_then(|p| p.cover.clone()).map(PathBuf::from).or_else(|| first.and_then(|t| app.cover_path(&t, cx)));
            let group: gpui::SharedString = format!("ppl-g-{i}").into();
            let drag = PlDrag { from: i, name: name.clone().into(), count, art: cover_path, width };
            rows.push(
                pl_row_inner(div().flex_none().size(px(40.0)).rounded(px(6.0)).overflow_hidden().child(art).into_any_element(), &name, count, group, false)
                    .id(("profile-pl-row", i))
                    .rounded(px(8.0))
                    .cursor_grab()
                    .on_drag(drag, |d, _, _, cx| cx.new(|_| d.clone()))
                    .child(switch(("profile-pl", i), on).on_click(cx.listener(move |this, _, _, cx| this.profile_toggle_playlist(id.clone(), cx))))
                    .into_any_element(),
            );
        }
        let end = gaps.get(slot).copied().unwrap_or(0.0);
        if end > 0.5 {
            rows.push(div().flex_none().h(px(end)).into_any_element());
        }
        div()
            .id("profile-pl-list")
            .flex()
            .flex_col()
            .pt(px(6.0))
            .on_drag_move::<PlDrag>(cx.listener(move |this, e: &gpui::DragMoveEvent<PlDrag>, _, cx| {
                let from = e.drag(cx).from;
                let y = e.event.position.y - e.bounds.top() - px(6.0);
                let slot = ((f32::from(y) / PL_ROW_H).floor().max(0.0) as usize).min(n.saturating_sub(1));
                if this.profile.pl_drop != Some((from, slot)) {
                    this.profile.pl_drop = Some((from, slot));
                    cx.notify();
                }
            }))
            .on_drop(cx.listener(move |this, d: &PlDrag, _, cx| {
                let to = this.profile.pl_drop.take().map_or(d.from, |(_, slot)| slot);
                this.profile_reorder_playlist(&order, d.from, to, cx);
                cx.notify();
            }))
            .children(rows)
            .into_any_element()
    };
    col.child(top)
        .child(section_label(t("profile.playlists.section")))
        .child(div().pt(px(2.0)).text_size(px(12.5)).text_color(white(0.5)).child(t(if public { "profile.playlists.desc" } else { "profile.playlists.desc_private" })))
        .child(list)
}


/// Height of one playlist row in Settings -> Privacy; drop slots use it.
const PL_ROW_H: f32 = 56.0;

/// The dragged profile playlist row: the row itself follows the cursor.
#[derive(Clone)]
struct PlDrag {
    from: usize,
    name: gpui::SharedString,
    count: usize,
    art: Option<PathBuf>,
    width: gpui::Pixels,
}

/// Grip, cover, name + song count: shared by the row and its drag image.
fn pl_row_inner(art: AnyElement, name: &str, count: usize, group: gpui::SharedString, lifted: bool) -> Div {
    let hk = hover_k(&group);
    div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .h(px(PL_ROW_H))
        .px(px(6.0))
        .relative()
        .when(!lifted, |d| d.child(hover_fill(group, white(0.04), 8.0)))
        .child(div().flex_none().opacity(if lifted { 1.0 } else { 0.4 + 0.6 * hk }).child(icon("grip-vertical", 15.0, white(0.6))))
        .child(art)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .child(div().truncate().text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(gpui::white()).child(name.to_string()))
                .child(div().text_size(px(12.5)).text_color(white(0.55)).child(tf("common.n_songs", &[("n", &count)]))),
        )
}

impl gpui::Render for PlDrag {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        theme::on_panel(|| {
            let art = match self.art.clone() {
                Some(p) => img(p).flex_none().size(px(40.0)).rounded(px(6.0)).object_fit(ObjectFit::Cover).into_any_element(),
                None => div().flex_none().size(px(40.0)).rounded(px(6.0)).bg(white(0.08)).into_any_element(),
            };
            pl_row_inner(art, &self.name, self.count, "ppl-lift".into(), true)
                .w(self.width)
                .rounded(px(8.0))
                .bg(gpui::hsla(0.0, 0.0, 0.16, 1.0))
                .border_1()
                .border_color(white(0.10))
                .font_family(theme::FONT)
                .cursor_grabbing()
        })
    }
}

/// Crop dialog for a picked profile picture or banner: drag the picture,
/// zoom with the slider or the wheel, then apply.
pub fn crop_dialog(app: &mut Glyph, cx: &mut Context<Glyph>) -> AnyElement {
    use gpui::{deferred, MouseButton};
    let Some(c) = app.profile.crop.as_ref() else { return div().into_any_element() };
    let dim = gpui::hsla(0.0, 0.0, 0.0, 0.6);
    let (sw, sh) = CROP_STAGE;
    let (ww, wh) = c.window();
    let (vx, vy) = ((sw - ww) / 2.0, (sh - wh) / 2.0);
    let s = c.scale();
    let (x, y, _, _) = crop_rect(c.size.0, c.size.1, c.banner, c.crop);
    let (iw, ih) = (c.size.0 as f32 * s, c.size.1 as f32 * s);
    let frac = (c.crop.zoom - 1.0) / (CROP_MAX_ZOOM - 1.0);
    let banner = c.banner;
    let shade = |l: f32, t: f32, w: f32, h: f32| div().absolute().left(px(l)).top(px(t)).w(px(w.max(0.0))).h(px(h.max(0.0))).bg(dim);
    let frame = div()
        .absolute()
        .left(px(vx))
        .top(px(vy))
        .w(px(ww))
        .h(px(wh))
        .border_2()
        .border_color(white(0.95))
        .when(banner, |d| d.rounded(px(8.0)))
        .when(!banner, |d| d.rounded_full());
    let stage = div()
        .id("crop-stage")
        .relative()
        .w(px(sw))
        .h(px(sh))
        .rounded(px(8.0))
        .overflow_hidden()
        .bg(white(0.04))
        .cursor(gpui::CursorStyle::OpenHand)
        .child(img(c.path.clone()).absolute().left(px(vx - x as f32 * s)).top(px(vy - y as f32 * s)).w(px(iw)).h(px(ih)).object_fit(ObjectFit::Fill))
        .child(shade(0.0, 0.0, sw, vy))
        .child(shade(0.0, vy + wh, sw, sh - vy - wh))
        .child(shade(0.0, vy, vx, wh))
        .child(shade(vx + ww, vy, sw - vx - ww, wh))
        .child(frame)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, e: &gpui::MouseDownEvent, _, cx| {
                if let Some(c) = this.profile.crop.as_mut() {
                    c.drag = Some(((f32::from(e.position.x), f32::from(e.position.y)), (c.crop.cx, c.crop.cy)));
                }
                cx.stop_propagation();
            }),
        )
        .on_mouse_move(cx.listener(|this, e: &gpui::MouseMoveEvent, _, cx| {
            let Some(c) = this.profile.crop.as_mut() else { return };
            let Some(((mx, my), (ox, oy))) = c.drag else { return };
            if e.pressed_button != Some(MouseButton::Left) {
                c.drag = None;
                return;
            }
            let s = c.scale();
            let (iw, ih) = (c.size.0 as f32 * s, c.size.1 as f32 * s);
            c.crop.cx = ox - (f32::from(e.position.x) - mx) / iw.max(1.0);
            c.crop.cy = oy - (f32::from(e.position.y) - my) / ih.max(1.0);
            c.clamp();
            cx.notify();
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                if let Some(c) = this.profile.crop.as_mut() {
                    c.drag = None;
                }
                cx.notify();
            }),
        )
        .on_scroll_wheel(cx.listener(|this, e: &gpui::ScrollWheelEvent, _, cx| {
            let dy = f32::from(e.delta.pixel_delta(px(20.0)).y);
            let z = this.profile.crop.as_ref().map_or(1.0, |c| c.crop.zoom);
            this.profile_crop_zoom(z * (1.0 + dy / 400.0), cx);
        }));
    let view = cx.entity().downgrade();
    let pick: Pick = Rc::new(move |f, _, app| {
        view.update(app, |s, cx| s.profile_crop_zoom(1.0 + f * (CROP_MAX_ZOOM - 1.0), cx)).ok();
    });
    let slider = div()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(12.0))
        .child(icon("image", 14.0, white(0.6)))
        .child(thin_bar("crop-zoom", 220.0, frac, pick, None, cx))
        .child(icon("image", 20.0, white(0.6)));
    let panel = div()
        .id("crop-dialog")
        .occlude()
        .w(px(sw + 48.0))
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
                .child(
                    div()
                        .flex_1()
                        .text_size(px(20.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(white(0.95))
                        .child(t(if banner { "profile.crop.banner" } else { "profile.crop.avatar" })),
                )
                .child(icon_button("crop-close", "x", 18.0, white(0.6)).on_click(cx.listener(|this, _, _, cx| this.profile_crop_cancel(cx)))),
        )
        .child(stage)
        .child(slider)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    text_button("crop-reset", t("profile.crop.reset"))
                        .h(px(36.0))
                        .text_size(theme::text_sm())
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(c) = this.profile.crop.as_mut() {
                                c.crop = Crop::default();
                            }
                            cx.notify();
                        })),
                )
                .child(div().flex_1())
                .child(button("crop-cancel", t("dialog.cancel")).h(px(36.0)).px(px(18.0)).on_click(cx.listener(|this, _, _, cx| this.profile_crop_cancel(cx))))
                .child(
                    primary_button("crop-apply", t("profile.crop.apply"))
                        .h(px(36.0))
                        .px(px(18.0))
                        .rounded(px(8.0))
                        .on_click(cx.listener(|this, _, _, cx| this.profile_crop_apply(cx))),
                ),
        );
    deferred(
        crate::ui::widgets::modal_layer(
            0.55,
            crate::ui::widgets::modal_backdrop("crop-backdrop", 0.55)
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.profile_crop_cancel(cx)))
                .child(panel),
        ),
    )
    .with_priority(5)
    .into_any_element()
}
