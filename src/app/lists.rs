use super::*;
use glyph_core::config::ListSort;

/// Display order of `tracks` under `sort`: indexes into `tracks`. Stable,
/// so equal titles keep the list's own order; missing albums and lengths go last.
pub fn sort_order(tracks: &[Track], sort: ListSort) -> Vec<usize> {
    let mut order: Vec<usize> = (0..tracks.len()).collect();
    let text = |s: &str| s.trim().to_lowercase();
    match sort {
        ListSort::Custom => {}
        ListSort::Title => order.sort_by_cached_key(|&i| text(&tracks[i].title)),
        ListSort::Artist => order.sort_by_cached_key(|&i| (text(&tracks[i].artist), tracks[i].album.as_deref().map(text), text(&tracks[i].title))),
        ListSort::Album => order.sort_by_cached_key(|&i| (tracks[i].album.is_none(), tracks[i].album.as_deref().map(text), text(&tracks[i].artist))),
        ListSort::Duration => order.sort_by_key(|&i| (tracks[i].duration_ms.is_none(), tracks[i].duration_ms)),
    }
    order
}

impl Glyph {
    // ------------------------------------------------------------ sorting

    pub fn list_sort(&self, l: &ListRef) -> ListSort {
        l.sort_id().and_then(|id| self.config.lists.sort.get(&id).copied()).unwrap_or_default()
    }

    pub fn set_list_sort(&mut self, l: &ListRef, sort: ListSort, cx: &mut Context<Self>) {
        let Some(id) = l.sort_id() else { return };
        if sort == ListSort::Custom {
            self.config.lists.sort.remove(&id);
        } else {
            self.config.lists.sort.insert(id, sort);
        }
        // Saved quietly (no "settings saved" toast for a sort).
        let toast = self.config_dirty.is_some_and(|(_, t)| t);
        self.config_dirty = Some((Instant::now(), toast));
        self.selected_row = None;
        cx.notify();
    }

    /// The list's songs in the order the list view shows (and plays) them.
    pub fn shown_tracks(&self, l: &ListRef) -> Vec<Track> {
        let tracks = self.list_tracks(l);
        match self.list_sort(l) {
            ListSort::Custom => tracks,
            s => sort_order(&tracks, s).into_iter().map(|i| tracks[i].clone()).collect(),
        }
    }

    /// Removes the song shown at row `i` (the rows may be sorted); the toast
    /// offers to put it back.
    pub fn remove_shown(&mut self, l: &ListRef, i: usize, cx: &mut Context<Self>) {
        let tracks = self.list_tracks(l);
        let i = match self.list_sort(l) {
            ListSort::Custom => Some(i),
            s => sort_order(&tracks, s).get(i).copied(),
        };
        let Some((i, track)) = i.and_then(|i| Some((i, tracks.get(i)?.clone()))) else { return };
        let same = |t: &Track| t.source == track.source && t.id == track.id;
        let undo = match l {
            ListRef::Liked | ListRef::ArtistLiked(_) => {
                self.library.liked.iter().position(same).map(|index| Undo::Liked { index, track: track.clone() })
            }
            ListRef::Recent => self.library.recent_song_index(&track).map(|index| Undo::Recent { index, track: track.clone() }),
            ListRef::User(id) => Some(Undo::PlaylistSong { id: id.clone(), index: i, track: track.clone() }),
            ListRef::Downloads => Some(Undo::Downloads(vec![track.clone()])),
            ListRef::Album { .. } => None,
        };
        let name = self.list_title(l);
        self.remove_from_list(l, i, cx);
        let Some(undo) = undo else { return };
        if self.list_tracks(l).iter().any(same) {
            return;
        }
        // Downloads: the same text as the toast the removal already showed,
        // so that one gets the button instead of a second toast.
        let msg = match l {
            ListRef::Downloads => t("toast.dl_removed_one").to_string(),
            _ => format!("{} · {}", tf("toast.removed_from", &[("name", &name)]), track.title),
        };
        self.toast_undo(msg, undo);
        cx.notify();
    }

    // ------------------------------------------------------------ lists & artists

    pub(super) fn route(&self) -> Route {
        Route { view: self.view, list: self.open_list.clone(), artist: self.open_artist.clone(), profile: self.open_profile.clone() }
    }

    /// "Show all" of a Home section: just its songs on a page.
    pub fn open_section(&mut self, title: String, tracks: Vec<Track>, cx: &mut Context<Self>) {
        self.push_route();
        self.view = View::Section;
        self.section = Some((title, tracks));
        self.selected_row = None;
        self.main_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        self.ctx_menu = None;
        cx.notify();
    }

    pub(crate) fn push_route(&mut self) {
        // Something opened from the results: the query was not left unpicked.
        if self.view == View::Search {
            self.search.unpicked = None;
        }
        let r = self.route();
        if self.history.last() != Some(&r) {
            self.history.push(r);
        }
        if self.history.len() > 50 {
            self.history.remove(0);
        }
    }

    pub(super) fn restore(&mut self, r: Route, cx: &mut Context<Self>) {
        self.view = r.view;
        self.selected_row = None;
        self.main_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        self.open_list = r.list;
        self.open_artist = r.artist.clone();
        if let Some(a) = r.artist {
            self.load_artist(a, cx);
        }
        self.open_profile = r.profile.clone();
        if let (View::Profile, Some(h)) = (r.view, r.profile) {
            self.load_profile(h, cx);
        }
        self.ctx_menu = None;
        cx.notify();
    }

    pub fn go_back(&mut self, cx: &mut Context<Self>) {
        if let Some(r) = self.history.pop() {
            self.restore(r, cx);
        }
    }

    /// Opens a song list (never starts playback).
    pub fn open_list(&mut self, l: ListRef, cx: &mut Context<Self>) {
        self.shared_preview = None;
        if self.view == View::List && self.open_list.as_ref() == Some(&l) {
            return;
        }
        self.push_route();
        self.view = View::List;
        self.selected_row = None;
        self.main_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        if self.library_ok && !self.nav_quiet && guard(|| self.library.push_recent_list(l.key())).is_some() {
            self.library.record_list_use(&l.key(), 0.25, glyph_core::paths::now_unix());
            self.save_library_activity();
        }
        if let Some(b) = &self.backend {
            let first = self.list_tracks(&l).into_iter().take(2).collect();
            guard(|| b.warm(first));
        }
        self.open_list = Some(l);
        self.open_artist = None;
        self.ctx_menu = None;
        cx.notify();
    }

    pub fn list_tracks(&self, l: &ListRef) -> Vec<Track> {
        match l {
            ListRef::Liked => self.library.liked.clone(),
            ListRef::Recent => self.library.recent_tracks().cloned().collect(),
            ListRef::User(id) => self.library.playlist(id).map(|p| p.tracks.clone()).unwrap_or_default(),
            ListRef::Album { tracks, .. } => tracks.clone(),
            ListRef::ArtistLiked(name) => self.liked_by(name),
            ListRef::Downloads => self.backend.as_ref().map(|b| b.downloaded_tracks()).unwrap_or_default(),
        }
    }

    /// Copies a short link for the list (public server) or its share code
    /// (songs, name) to the clipboard.
    pub fn share_list(&mut self, l: &ListRef, cx: &mut Context<Self>) {
        let tracks = self.list_tracks(l);
        if tracks.is_empty() {
            self.toast_info(t("toast.share_empty"));
            cx.notify();
            return;
        }
        let title = self.list_title(l);
        // Artist albums carry no author; a list opened from a link keeps its link.
        let kind = if self.shared_preview.is_some() { "playlist" } else if matches!(l, ListRef::Album { .. }) { "album" } else { "playlist" };
        let origin = if let Some((_, link, _)) = &self.shared_preview { Some(link.clone()) } else { match l {
            ListRef::User(id) => self.library.playlist(id).and_then(|p| p.origin.clone()),
            _ => None,
        }};
        self.share_songs(title, tracks, kind, origin, cx);
    }

    /// Copies a short link (public server) or a share code for `tracks`
    /// under `title`.
    pub fn share_songs(&mut self, title: String, tracks: Vec<Track>, kind: &'static str, origin: Option<String>, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else {
            let code = glyph_core::share::encode(&title, &tracks);
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(code));
            self.toast_sub(t("toast.share_copied"), title);
            cx.notify();
            return;
        };
        let signed_in = b.account().is_some();
        cx.spawn(async move |this, cx| {
            let text = b.share_text(&title, &tracks, kind, origin).await;
            this.update(cx, |s, cx| {
                if signed_in && glyph_core::share::link_id(&text).is_none() {
                    s.toast_info(t("toast.share_link_failed"));
                    cx.notify();
                    return;
                }
                let key = if glyph_core::share::link_id(&text).is_some() { "toast.share_link_copied" } else { "toast.share_copied" };
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                s.toast_sub(t(key), title);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Imports a pasted share code (or short link) as a new playlist and opens it.
    pub fn import_share_code(&mut self, code: &str, cx: &mut Context<Self>) -> bool {
        if glyph_core::share::link_id(code).is_some() {
            let Some(b) = self.backend.clone() else {
                self.toast_info(t("status.library_offline"));
                cx.notify();
                return true;
            };
            let link = format!("https://glyph.my/p/{}", glyph_core::share::link_id(code).unwrap());
            cx.spawn(async move |this, cx| {
                let r = b.resolve_share_link(&link).await;
                this.update(cx, |s, cx| match r {
                    Ok(p) if glyph_core::share::is_code(&p.code) => {
                        let author = p.author.filter(|_| p.kind != "album");
                        match glyph_core::share::decode(&p.code) {
                            Ok((name, _)) if p.kind == "artist" => s.open_artist(&name, cx),
                            Ok((name, tracks)) => {
                                let quiet = s.nav_quiet;
                                s.nav_quiet = true;
                                s.open_list(ListRef::Album { title: name, artist: String::new(), tracks }, cx);
                                s.nav_quiet = quiet;
                                s.shared_preview = Some((p.code, link.clone(), author));
                                cx.notify();
                            }
                            Err(_) => { s.toast_info(t("toast.share_link_failed")); cx.notify(); }
                        }
                    }
                    Ok(_) => {
                        s.toast_info(t("toast.share_link_failed"));
                        cx.notify();
                    }
                    Err(e) => {
                        tracing::info!("playlist link failed: {e:#}");
                        s.toast_info(t("toast.share_link_failed"));
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
            return true;
        }
        if !glyph_core::share::is_code(code) {
            return false;
        }
        self.import_code(code, None, None, cx);
        true
    }

    /// Adds the playlist of a share code; `origin` / `shared_by` come from a
    /// short link (its URL and the author's handle).
    pub(super) fn import_code(&mut self, code: &str, origin: Option<String>, shared_by: Option<String>, cx: &mut Context<Self>) {
        match glyph_core::share::decode(code) {
            Ok((name, tracks)) if self.library_ok => {
                if let Some(id) = origin.as_ref().and_then(|link| self.library.playlists.iter().find(|p| p.origin.as_ref() == Some(link)).map(|p| p.id.clone())) {
                    self.open_list(ListRef::User(id), cx);
                    return;
                }
                let n = tracks.len();
                let id = {
                    let p = self.library.create_playlist(&name);
                    p.tracks = tracks;
                    p.origin = origin;
                    p.shared_by = shared_by;
                    p.id.clone()
                };
                self.save_library(cx);
                self.toast_sub(tf("toast.share_imported", &[("n", &n)]), name);
                self.open_list(ListRef::User(id), cx);
            }
            Ok(_) => self.toast_info(t("status.library_offline")),
            Err(e) => self.toast_info(format!("{e:#}")),
        }
        cx.notify();
    }

    /// "2019 · EP"-style label for an album; looked up once, guessed from the
    /// song count until (or unless) the catalog knows it.
    pub fn album_label(&mut self, title: &str, artist: &str, n: usize, cx: &mut Context<Self>) -> String {
        use glyph_core::catalog::AlbumKind;
        let key = format!("{}|{}", title.to_lowercase(), artist.to_lowercase());
        if !self.album_meta.contains_key(&key) {
            if self.album_meta.len() >= 512 {
                if let Some(evict) = self.album_meta.keys().next().cloned() {
                    self.album_meta.remove(&evict);
                }
            }
            self.album_meta.insert(key.clone(), None);
            if let Some(b) = self.backend.clone() {
                let (t2, a2, k2) = (title.to_string(), artist.to_string(), key.clone());
                cx.spawn(async move |this, cx| {
                    let r = guard_async(b.album_meta(&t2, &a2)).await;
                    if let Ok(Some(m)) = r {
                        this.update(cx, |s, cx| {
                            s.album_meta.insert(k2, Some(m));
                            cx.notify();
                        })
                        .ok();
                    }
                })
                .detach();
            }
        }
        let meta = self.album_meta.get(&key).cloned().flatten();
        let kind = meta.as_ref().map(|m| m.kind).unwrap_or(AlbumKind::from_count(n));
        let kind = match kind {
            AlbumKind::Album => t("album.kind.album"),
            AlbumKind::Ep => t("album.kind.ep"),
            AlbumKind::Single => t("album.kind.single"),
            AlbumKind::Compilation => t("album.kind.compilation"),
        };
        match meta.and_then(|m| m.year) {
            Some(y) => format!("{y} · {kind}"),
            None => kind.to_string(),
        }
    }

    /// The taste profile from the library (cheap; built on demand).
    pub fn taste(&self) -> glyph_core::taste::Taste {
        if self.library_ok {
            glyph_core::taste::Taste::from_library(&self.library)
        } else {
            Default::default()
        }
    }

    /// Not filtered out by "Hide explicit songs".
    pub fn allowed(&self, t: &Track) -> bool {
        !(t.maybe_explicit() && self.config.audio.hide_explicit)
    }

    /// Loads "New from artists you follow" for Home: releases of the last
    /// eight weeks. Cached for 12 hours per follow list.
    pub fn load_new_releases(&mut self, cx: &mut Context<Self>) {
        if !self.library_ok || self.account.merge_pending { return; }
        let scope = self.library.storage_path.0.clone().unwrap_or_else(Library::guest_path);
        let epoch = self.account.data_epoch;
        const ARTISTS: usize = 40;
        const DAYS: u32 = 56;
        let Some(b) = self.backend.clone() else { return };
        if self.new_releases_loading {
            return;
        }
        let artists: Vec<String> = self.library.followed.iter().take(ARTISTS).map(|a| a.name.clone()).collect();
        if artists.is_empty() {
            self.new_releases.clear();
            return;
        }
        if let Some(cached) = load_releases_cache(&scope, &artists, 12 * 3600) {
            self.new_releases = cached;
            cx.notify();
            return;
        }
        self.new_releases_loading = true;
        cx.spawn(async move |this, cx| {
            let found = b.new_releases(artists.clone(), DAYS).await;
            this.update(cx, |s, cx| {
                if s.account.data_epoch != epoch { return; }
                s.new_releases_loading = false;
                if let Ok(list) = found {
                    save_releases_cache(&scope, &artists, &list);
                    s.new_releases = list.into_iter().filter(|r| !s.is_hidden_artist(&r.artist)).take(20).collect();
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Follow / unfollow: only that artist's releases join or leave the row
    /// (no reload of the whole follow list).
    pub(super) fn new_releases_follow_changed(&mut self, name: &str, now: bool, cx: &mut Context<Self>) {
        let scope = self.library.storage_path.0.clone().unwrap_or_else(Library::guest_path);
        let epoch = self.account.data_epoch;
        const DAYS: u32 = 56;
        let artists: Vec<String> = self.library.followed.iter().take(40).map(|a| a.name.clone()).collect();
        if !now {
            self.new_releases.retain(|r| !r.artist.eq_ignore_ascii_case(name));
            save_releases_cache(&scope, &artists, &self.new_releases);
            cx.notify();
            return;
        }
        let Some(b) = self.backend.clone() else { return };
        let name = name.to_string();
        cx.spawn(async move |this, cx| {
            let Ok(found) = b.new_releases(vec![name], DAYS).await else { return };
            this.update(cx, |s, cx| {
                if s.account.data_epoch != epoch { return; }
                for r in found {
                    if !s.new_releases.iter().any(|x| x.id == r.id) {
                        s.new_releases.push(r);
                    }
                }
                s.new_releases.sort_by(|a, b| b.released.cmp(&a.released));
                s.new_releases.truncate(20);
                save_releases_cache(&scope, &artists, &s.new_releases);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Loads "Recommended for you" and "Popular right now" for Home.
    pub fn load_feeds(&mut self, cx: &mut Context<Self>) {
        if !self.library_ok || self.account.merge_pending { return; }
        let scope = self.library.storage_path.0.clone().unwrap_or_else(Library::guest_path);
        let epoch = self.account.data_epoch;
        let Some(b) = self.backend.clone() else { return };
        if self.feeds_loading {
            return;
        }
        // Data saver: reuse the last feeds for up to 12 hours.
        if self.config.storage.data_saver {
            if let Some((trending, recs)) = load_feeds_cache(&scope, 12 * 3600) {
                self.trending = trending;
                self.recs = recs;
                cx.notify();
                return;
            }
        }
        self.feeds_loading = true;
        // Seeds: one song for each of the user's top artists, then liked songs.
        let taste = self.taste();
        let mut seeds: Vec<Track> = Vec::new();
        let pool: Vec<Track> = self.library.liked.iter().chain(self.library.recent_tracks()).cloned().collect();
        for a in taste.top_artists(4) {
            if let Some(t) = pool.iter().find(|t| glyph_core::taste::credited(&t.artist).contains(&a)) {
                if !seeds.iter().any(|s| s.id == t.id && s.source == t.source) {
                    seeds.push(t.clone());
                }
            }
        }
        for t in self.library.liked.iter().take(2) {
            if !seeds.iter().any(|s| s.id == t.id && s.source == t.source) {
                seeds.push(t.clone());
            }
        }
        seeds.truncate(5);
        cx.spawn(async move |this, cx| {
            let trending = b.trending(30).await.unwrap_or_default();
            this.update(cx, |s, cx| {
                if s.account.data_epoch != epoch { return; }
                s.trending = trending.into_iter().filter(|t| s.allowed(t) && !s.is_hidden_artist(&t.artist)).take(20).collect();
                cx.notify();
            })
            .ok();
            // Similar songs per seed (via the server); songs suggested by
            // several seeds and by liked artists come first.
            let mut found: Vec<(Track, f32)> = Vec::new();
            for seed in &seeds {
                if !this.update(cx, |s, _| s.account.data_epoch == epoch).unwrap_or(false) { return; }
                let sim = b.similar(seed).await.unwrap_or_default();
                for (i, t) in sim.into_iter().take(25).enumerate() {
                    let w = 1.0 - i as f32 / 30.0;
                    let key = glyph_core::dedupe::track_key(&t);
                    match found.iter_mut().find(|(x, _)| glyph_core::dedupe::track_key(x) == key) {
                        Some(f) => f.1 += w,
                        None => found.push((t, w)),
                    }
                }
            }
            this.update(cx, |s, cx| {
                if s.account.data_epoch != epoch { return; }
                let taste = s.taste();
                let recent: HashSet<String> = s.library.recent_tracks().take(30).map(glyph_core::dedupe::track_key).collect();
                let mut v: Vec<(Track, f32)> = found
                    .into_iter()
                    .filter(|(t, _)| s.allowed(t) && !taste.is_hidden(&t.artist) && !taste.is_liked(t) && !recent.contains(&glyph_core::dedupe::track_key(t)))
                    .map(|(t, w)| {
                        let preference = s.library.recommendation_feedback.get(&t.artist.trim().to_lowercase()).copied().unwrap_or(0);
                        let sc = w + taste.artist_weight(&t.artist) * 0.8 + preference as f32 * 2.0;
                        (t, sc)
                    })
                    .collect();
                v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                // At most two songs per artist, so it does not become one artist's page.
                let mut per: std::collections::HashMap<String, usize> = Default::default();
                s.recs = v
                    .into_iter()
                    .filter(|(t, _)| {
                        let n = per.entry(glyph_core::dedupe::normalize_artist(&t.artist)).or_default();
                        *n += 1;
                        *n <= 2
                    })
                    .map(|(t, _)| t)
                    .take(16)
                    .collect();
                s.feeds_loading = false;
                save_feeds_cache(&scope, &s.trending, &s.recs);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
