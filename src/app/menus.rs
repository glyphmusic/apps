use super::*;

impl Glyph {
    /// Jump list / dock entry `n`: 0..=4 recently played, 5 play/pause,
    /// 6 next, 7 previous.
    pub(super) fn dock_action(&mut self, n: usize, cx: &mut Context<Self>) {
        match n {
            0..=4 => self.play_recent(n, cx),
            5 => self.toggle_play(cx),
            6 => self.next(false, cx),
            7 => self.previous(cx),
            _ => {}
        }
    }

    /// Recently played songs + transport in the taskbar jump list (Windows)
    /// and the dock menu (macOS). Linux has no such menu; nothing happens there.
    pub(super) fn update_dock_menu(&mut self, cx: &mut Context<Self>) {
        if !self.library_ok {
            return;
        }
        let keys: Vec<String> = self.library.recent_tracks().take(5).map(track_key).collect();
        if keys == self.dock_keys {
            return;
        }
        self.dock_keys = keys;
        // Windows: own jump list with a "Recently played" category.
        #[cfg(windows)]
        {
            use crate::jumplist::{update, Entry};
            let recent = self
                .library
                .recent_tracks()
                .take(5)
                .enumerate()
                .map(|(i, tr)| Entry { label: format!("{} · {}", tr.title, tr.artist), action: i })
                .collect();
            let tasks = vec![
                Entry { label: t("dock.play_pause").to_string(), action: 5 },
                Entry { label: t("dock.next").to_string(), action: 6 },
                Entry { label: t("dock.previous").to_string(), action: 7 },
            ];
            update(t("library.recent").to_string(), recent, tasks);
            let _ = cx;
        }
        // macOS: dock menu with in-process actions.
        #[cfg(not(windows))]
        self.set_platform_dock_menu(cx);
    }

    #[cfg(not(windows))]
    pub(super) fn set_platform_dock_menu(&mut self, cx: &mut Context<Self>) {
        use crate::ui::{NextTrack, PlayRecent0, PlayRecent1, PlayRecent2, PlayRecent3, PlayRecent4, PrevTrack, TogglePlay};
        let mut items: Vec<gpui::MenuItem> = Vec::new();
        for (i, tr) in self.library.recent_tracks().take(5).enumerate() {
            let name = format!("{} · {}", tr.title, tr.artist);
            items.push(match i {
                0 => gpui::MenuItem::action(name, PlayRecent0),
                1 => gpui::MenuItem::action(name, PlayRecent1),
                2 => gpui::MenuItem::action(name, PlayRecent2),
                3 => gpui::MenuItem::action(name, PlayRecent3),
                _ => gpui::MenuItem::action(name, PlayRecent4),
            });
        }
        items.push(gpui::MenuItem::action(t("dock.play_pause"), TogglePlay));
        items.push(gpui::MenuItem::action(t("dock.next"), NextTrack));
        items.push(gpui::MenuItem::action(t("dock.previous"), PrevTrack));
        cx.set_dock_menu(items);
    }

    /// Plays `tracks` from a random song with shuffle on.
    pub fn play_shuffled(&mut self, tracks: Vec<Track>, cx: &mut Context<Self>) {
        let tracks: Vec<Track> = tracks.into_iter().filter(|t| self.playable(t)).collect();
        if tracks.is_empty() {
            return;
        }
        let recent = self.recent_keys();
        self.playing_from = if self.view == View::List { self.open_list.clone() } else { None };
        self.queue_from = self.context_label();
        self.recent_artist_ctx = (self.view == View::Artist).then(|| self.open_artist.clone()).flatten();
        let mode = self.shuffle_mode();
        // The whole list is dealt, the first song included (smart shuffle
        // keeps a song played a moment ago from opening the list again).
        let Some(Some(track)) = guard(|| {
            self.queue.set_recent(recent);
            self.queue.set_shuffle_mode(mode);
            self.queue.set_shuffled(tracks).cloned()
        }) else {
            return;
        };
        self.pending_recent_artist = self.artist_recent_context(&track);
        self.play_track(track, cx);
    }

    /// Adds every song to the end of the queue.
    pub fn queue_all(&mut self, tracks: Vec<Track>, cx: &mut Context<Self>) {
        let tracks: Vec<Track> = tracks.into_iter().filter(|t| self.playable(t)).collect();
        let n = tracks.len();
        for tr in tracks {
            guard(|| self.queue.push(tr));
        }
        self.queue_changed();
        self.toast(tf("toast.queued_n", &[("n", &n)]));
        cx.notify();
    }

    pub fn open_ctx(&mut self, track: Track, pos: gpui::Point<gpui::Pixels>, from: Option<(ListRef, usize)>, cx: &mut Context<Self>) {
        self.ctx_menu = Some(CtxMenu { track, pos, from, playlists: false, sub: 0, list: None, artist: None, sidebar: None });
        self.ctx_opened = Instant::now();
        self.ctx_closing = None;
        cx.notify();
    }

    /// "More" menu of an artist page.
    pub fn open_artist_ctx(&mut self, name: &str, pos: gpui::Point<gpui::Pixels>, cx: &mut Context<Self>) {
        let songs = self.artists.get(name).map(|p| p.tracks.clone()).unwrap_or_default();
        let track = songs.first().cloned().unwrap_or(Track {
            id: String::new(),
            source: ProviderId::Audius,
            title: String::new(),
            artist: name.to_string(),
            album: None,
            duration_ms: None,
            cover_url: None,
            isrc: None,
            permalink: None,
            external_ids: Default::default(),
            explicit: false,
        });
        self.ctx_menu = Some(CtxMenu { track, pos, from: None, playlists: false, sub: 0, list: None, artist: Some(name.to_string()), sidebar: None });
        self.ctx_opened = Instant::now();
        self.ctx_closing = None;
        cx.notify();
    }

    /// Signed-in users share a Glyph artist link; guests copy the Wikipedia article, else
    /// their top song on its platform.
    pub fn copy_artist_link(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.backend.as_ref().is_some_and(|b| b.account().is_some()) {
            let tracks = self.artists.get(name).map(|p| p.tracks.clone()).unwrap_or_default();
            self.copy_glyph_link(name.to_string(), tracks, "artist", cx);
            return;
        }
        let page = self.artists.get(name);
        let url = page
            .and_then(|p| p.info.as_ref().map(|i| i.url.clone()).filter(|u| !u.is_empty()))
            .or_else(|| page.and_then(|p| p.tracks.iter().find_map(|t| t.permalink.clone())));
        match url {
            Some(u) => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(u));
                self.toast_sub(t("toast.link_copied_plain"), name.to_string());
            }
            None => self.toast_info(t("toast.no_link")),
        }
    }

    pub fn is_hidden_artist(&self, credit: &str) -> bool {
        self.library_ok && !self.library.hidden_artists.is_empty() && self.library.hidden_artists.iter().any(|h| by_artist(h, credit))
    }

    /// "Don't show this artist" / "Show again".
    pub fn toggle_hidden_artist(&mut self, name: &str, cx: &mut Context<Self>) {
        if !self.library_ok {
            return;
        }
        if let Some(hidden) = guard(|| self.library.toggle_hidden(name)) {
            self.save_library(cx);
            if hidden {
                let n = name.to_string();
                self.search.retain_results(|t| !by_artist(&n, &t.artist));
                self.toast(tf("toast.artist_hidden", &[("name", &name)]));
            } else {
                self.toast(tf("toast.artist_shown", &[("name", &name)]));
            }
        }
        cx.notify();
    }

    /// Right-click on an album / playlist tile.
    pub fn open_list_ctx(&mut self, l: ListRef, pos: gpui::Point<gpui::Pixels>, cx: &mut Context<Self>) {
        let first = self.list_tracks(&l).into_iter().next();
        let is_user = matches!(l, ListRef::User(_));
        // Empty lists only have something to offer when they are the user's own.
        let track = match first {
            Some(t) => t,
            None if is_user => Track {
                id: String::new(),
                source: ProviderId::Audius,
                title: String::new(),
                artist: String::new(),
                album: None,
                duration_ms: None,
                cover_url: None,
                isrc: None,
                permalink: None,
                external_ids: Default::default(),
                explicit: false,
            },
            None => return,
        };
        self.ctx_menu = Some(CtxMenu { track, pos, from: None, playlists: false, sub: 0, list: Some(l), artist: None, sidebar: None });
        self.ctx_opened = Instant::now();
        self.ctx_closing = None;
        cx.notify();
    }

    /// Right-click on a player control.
    pub fn open_control_menu(&mut self, kind: ControlMenu, pos: gpui::Point<gpui::Pixels>, cx: &mut Context<Self>) {
        if kind == ControlMenu::Volume {
            self.refresh_output_devices(cx);
            self.rescan_devices();
        }
        self.ctx_menu = None;
        self.control_closing = None;
        self.control_menu = Some((kind, pos, Instant::now()));
        cx.notify();
    }

    pub fn close_control_menu(&mut self, cx: &mut Context<Self>) {
        if let Some((kind, pos, _)) = self.control_menu.take() {
            self.control_closing = Some((kind, pos, Instant::now()));
            cx.notify();
        }
    }

    /// Opens the mini player as the card (`big`) or the slim bar; an open
    /// one switches.
    pub fn open_mini_as(&mut self, big: bool, cx: &mut Context<Self>) {
        if self.mini_shown {
            self.set_mini_big(big, cx);
        } else {
            self.set_mini_big_flag(big, cx);
            self.toggle_mini(cx);
        }
    }

    pub fn close_ctx(&mut self, cx: &mut Context<Self>) {
        if self.ctx_menu.is_some() {
            self.begin_ctx_close();
            cx.notify();
        }
    }

    /// Lets the open menu fade out (the caller notifies).
    pub fn begin_ctx_close(&mut self) {
        if let Some(m) = self.ctx_menu.take() {
            self.ctx_closing = Some((m, Instant::now()));
        }
    }

    /// Removes the song at `i` from a list (unlike for Liked Songs).
    pub fn remove_from_list(&mut self, l: &ListRef, i: usize, cx: &mut Context<Self>) {
        match l {
            ListRef::Liked => {
                if let Some(tr) = self.library.liked.get(i).cloned() {
                    self.toggle_like(&tr, cx);
                }
            }
            ListRef::Recent => {
                // The row index counts songs, so map it back to its entry.
                let tr = self.library.recent_tracks().nth(i).cloned();
                if let Some(tr) = tr {
                    self.library.remove_recent_song(&tr);
                    self.save_library(cx);
                }
            }
            ListRef::User(id) => {
                if guard(|| self.library.remove_from_playlist(id, i)).is_some() {
                    self.save_library(cx);
                }
            }
            ListRef::ArtistLiked(name) => {
                if let Some(tr) = self.liked_by(name).get(i).cloned() {
                    self.toggle_like(&tr, cx);
                }
            }
            ListRef::Album { .. } => {}
            ListRef::Downloads => {
                if let Some(tr) = self.list_tracks(l).get(i).cloned() {
                    self.remove_downloads(vec![tr], cx);
                }
            }
        }
        cx.notify();
    }

    /// Opens the name dialog for a new playlist (with `track` added) or for
    /// renaming playlist `rename`.
    pub fn open_name_dialog(&mut self, tracks: Vec<Track>, rename: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let current = rename.as_ref().and_then(|id| self.library.playlist(id)).map(|p| p.name.clone()).unwrap_or_default();
        self.playlist_name.update(cx, |i, cx| i.set_value(current, cx));
        // A running import keeps its link; a finished or failed one is cleared.
        if self.link_import.as_ref().is_none_or(|l| l.error.is_some()) {
            self.link_import = None;
            self.playlist_link.update(cx, |i, cx| i.set_value(String::new(), cx));
        }
        self.ctx_menu = None;
        let initial_playlist = rename.as_ref().and_then(|id| self.library.playlist(id).map(|p|
            (p.name.clone(), p.cover.clone(), self.is_auto_download(id),
                self.profile.settings.as_ref().map(|s| s.playlists.contains(id)))));
        self.name_dialog = Some(NameDialog { tracks, rename, eq_preset: false, initial_playlist });
        let f = self.playlist_name.read(cx).focus.clone();
        window.focus(&f, cx);
        cx.notify();
    }

    /// Lets the user pick a picture for playlist `id` (file dialog); it is
    /// copied into the data dir as a 512 px JPEG.
    pub fn pick_playlist_cover(&mut self, id: String, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t("dialog.playlist.pick").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(src) = paths.into_iter().next() else { return };
            let pid = id.clone();
            let saved = cx
                .background_executor()
                .spawn(async move {
                    let img = image::open(&src).map_err(|e| e.to_string())?;
                    // Square crop from the centre, 512 px.
                    let side = img.width().min(img.height());
                    let img = img.crop_imm((img.width() - side) / 2, (img.height() - side) / 2, side, side).thumbnail(512, 512);
                    let dir = glyph_core::paths::data_dir().join("playlist-covers");
                    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    // New name per change: GPUI caches images by path.
                    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
                    let path = dir.join(format!("{pid}-{stamp}.jpg"));
                    img.to_rgb8().save(&path).map_err(|e| e.to_string())?;
                    Ok::<_, String>(path)
                })
                .await;
            this.update(cx, |s, cx| {
                match saved {
                    Ok(path) => {
                        let old = s.library.playlist(&id).and_then(|p| p.cover.clone());
                        if guard(|| s.library.set_playlist_cover(&id, Some(path.display().to_string()))).is_some() {
                            s.save_library(cx);
                            if let Some(old) = old {
                                let _ = std::fs::remove_file(old);
                            }
                        }
                    }
                    Err(e) => s.toast_info(tf("toast.cover_error", &[("e", &e)])),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn remove_playlist_cover(&mut self, id: &str, cx: &mut Context<Self>) {
        let old = self.library.playlist(id).and_then(|p| p.cover.clone());
        if guard(|| self.library.set_playlist_cover(id, None)).is_some() {
            self.save_library(cx);
            if let Some(old) = old {
                let _ = std::fs::remove_file(old);
            }
        }
        cx.notify();
    }

    /// Opens (or closes, when open for it already) the "Add songs" panel of
    /// playlist `id`.
    pub fn toggle_add_songs(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.add_songs.as_ref().is_some_and(|a| a.open && a.id == id) {
            self.close_add_songs(cx);
            return;
        }
        self.show_queue = false;
        self.add_songs = Some(AddSongs {
            id: id.to_string(),
            open: true,
            tab: 0,
            sugg: Vec::new(),
            sugg_loading: false,
            query: String::new(),
            results: Vec::new(),
            searching: false,
            seq: 0,
            task: None,
        });
        self.add_input.update(cx, |i, cx| i.set_value(String::new(), cx));
        let f = self.add_input.read(cx).focus.clone();
        window.focus(&f, cx);
        self.load_add_suggestions(cx);
        cx.notify();
    }

    pub fn close_add_songs(&mut self, cx: &mut Context<Self>) {
        if let Some(a) = self.add_songs.as_mut() {
            a.open = false;
            a.task = None;
            cx.notify();
        }
    }

    /// Songs similar to the playlist's own (a few spread over it), minus
    /// what is in it already; liked songs seed an empty playlist.
    pub(super) fn load_add_suggestions(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let Some(id) = self.add_songs.as_ref().map(|a| a.id.clone()) else { return };
        let own: Vec<Track> = self.library.playlist(&id).map(|p| p.tracks.clone()).unwrap_or_default();
        let pool = if own.is_empty() { self.library.liked.clone() } else { own };
        let n = pool.len();
        let k = n.min(5);
        let mut seeds: Vec<Track> = (0..k).map(|i| pool[n - 1 - i * n / k].clone()).collect();
        seeds.dedup_by(|a, b| a.id == b.id && a.source == b.source);
        if seeds.is_empty() {
            // Nothing to go on: what Home recommends.
            let v: Vec<Track> = self.recs.iter().chain(self.trending.iter()).cloned().collect();
            if let Some(a) = self.add_songs.as_mut() {
                a.sugg = v;
            }
            return;
        }
        if let Some(a) = self.add_songs.as_mut() {
            a.sugg_loading = true;
        }
        cx.spawn(async move |this, cx| {
            let mut found: Vec<(Track, f32)> = Vec::new();
            for seed in &seeds {
                let sim = b.similar(seed).await.unwrap_or_default();
                for (i, t) in sim.into_iter().take(30).enumerate() {
                    let w = 1.0 - i as f32 / 40.0;
                    let key = glyph_core::dedupe::track_key(&t);
                    match found.iter_mut().find(|(x, _)| glyph_core::dedupe::track_key(x) == key) {
                        Some(f) => f.1 += w,
                        None => found.push((t, w)),
                    }
                }
            }
            found.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            this.update(cx, |s, cx| {
                if s.add_songs.as_ref().is_none_or(|a| a.id != id) {
                    return;
                }
                let have: HashSet<String> = s.library.playlist(&id).map(|p| p.tracks.iter().map(glyph_core::dedupe::track_key).collect()).unwrap_or_default();
                let mut per: std::collections::HashMap<String, usize> = Default::default();
                let v: Vec<Track> = found
                    .into_iter()
                    .map(|(t, _)| t)
                    .filter(|t| s.allowed(t) && !s.is_hidden_artist(&t.artist) && !have.contains(&glyph_core::dedupe::track_key(t)))
                    .filter(|t| {
                        let n = per.entry(glyph_core::dedupe::normalize_artist(&t.artist)).or_default();
                        *n += 1;
                        *n <= 3
                    })
                    .take(50)
                    .collect();
                if let Some(a) = s.add_songs.as_mut() {
                    a.sugg = v;
                    a.sugg_loading = false;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Search inside the "Add songs" panel (first page only, after a short pause).
    pub fn add_search(&mut self, q: String, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let Some(a) = self.add_songs.as_mut() else { return };
        if a.query == q {
            return;
        }
        a.query = q.clone();
        a.seq += 1;
        a.results.clear();
        let seq = a.seq;
        let plain = glyph_core::taste::parse_query(q.trim()).0;
        if plain.trim().is_empty() {
            a.searching = false;
            a.task = None;
            cx.notify();
            return;
        }
        a.searching = true;
        a.task = Some(cx.spawn(async move |this, cx| {
            use futures::StreamExt;
            cx.background_executor().timer(Duration::from_millis(300)).await;
            let Some(mut stream) = guard(|| b.search_page(&plain, 0)) else { return };
            while let Some(ev) = stream.next().await {
                let done = matches!(ev, SearchEvent::Done);
                let alive = this
                    .update(cx, |s, cx| {
                        let ok: Vec<Track> = match &ev {
                            SearchEvent::Results { tracks, .. } => tracks.iter().filter(|t| s.allowed(t) && !s.is_hidden_artist(&t.artist)).cloned().collect(),
                            _ => Vec::new(),
                        };
                        let Some(a) = s.add_songs.as_mut().filter(|a| a.seq == seq) else { return false };
                        for t in ok {
                            if a.results.len() < 50 && !a.results.iter().any(|x| x.id == t.id && x.source == t.source) {
                                a.results.push(t);
                            }
                        }
                        if done {
                            a.searching = false;
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if done || !alive {
                    break;
                }
            }
            this.update(cx, |s, cx| {
                if let Some(a) = s.add_songs.as_mut().filter(|a| a.seq == seq) {
                    a.searching = false;
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
    }

    pub fn set_add_tab(&mut self, tab: u8, cx: &mut Context<Self>) {
        if let Some(a) = self.add_songs.as_mut() {
            a.tab = tab;
            self.add_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
            cx.notify();
        }
    }

    /// Adds `t` to the panel's playlist, or takes it out again.
    pub fn add_songs_toggle(&mut self, t: Track, cx: &mut Context<Self>) {
        let Some(id) = self.add_songs.as_ref().map(|a| a.id.clone()) else { return };
        let at = self.library.playlist(&id).and_then(|p| p.tracks.iter().position(|x| x.id == t.id && x.source == t.source));
        match at {
            Some(i) => self.remove_from_list(&ListRef::User(id), i, cx),
            None => self.add_to_playlist(&id, t, cx),
        }
    }

    /// Drag-to-reorder in an own playlist.
    pub fn move_in_playlist(&mut self, id: &str, from: usize, to: usize, cx: &mut Context<Self>) {
        if from != to && guard(|| self.library.move_in_playlist(id, from, to)).is_some() {
            self.touch_list(&ListRef::User(id.to_string()), 0.3);
            self.save_library(cx);
        }
        cx.notify();
    }

    pub fn close_name_dialog(&mut self, cx: &mut Context<Self>) {
        if self.name_dialog.take().is_some() {
            cx.notify();
        }
    }

    pub fn playlist_edit_changed(&self, cx: &Context<Self>) -> bool {
        let Some(d) = &self.name_dialog else { return false; };
        let (Some(id), Some(initial)) = (&d.rename, &d.initial_playlist) else { return false; };
        let Some(p) = self.library.playlist(id) else { return false; };
        let name = self.playlist_name.read(cx).value.trim();
        !name.is_empty() && (name != initial.0 || p.cover != initial.1 || self.is_auto_download(id) != initial.2
            || self.profile.settings.as_ref().map(|s| s.playlists.contains(id)) != initial.3)
    }

    pub fn submit_name_dialog(&mut self, cx: &mut Context<Self>) {
        if self.name_dialog.as_ref().is_some_and(|d| d.rename.is_some()) && !self.playlist_edit_changed(cx) { return; }
        let Some(d) = self.name_dialog.take() else { return };
        let typed = self.playlist_name.read(cx).value.trim().to_string();
        if d.eq_preset {
            let name = glyph_core::eq::clean_name(&typed);
            if !name.is_empty() {
                let gains = eq_gains(&self.config.audio.eq_gains);
                self.add_eq_preset(glyph_core::eq::EqPreset::new(&name, gains), cx);
                self.toast_sub(t("toast.eq_saved"), name);
            }
        } else if let Some(id) = d.rename {
            self.touch_list(&ListRef::User(id.clone()), 0.3);
            if !typed.is_empty() && guard(|| self.library.rename_playlist(&id, &typed)).is_some() {
                self.save_library(cx);
            }
        } else if !self.playlist_link.read(cx).value.trim().is_empty() {
            // Import from the pasted link; the dialog stays open with progress.
            let link = self.playlist_link.read(cx).value.trim().to_string();
            self.name_dialog = Some(d);
            if self.link_import.as_ref().is_some_and(|l| l.error.is_none()) {
                return;
            }
            if glyph_core::import::parse_url(&link).is_none() {
                self.link_import_error(t("import.unknown_link").to_string());
            } else {
                self.link_import = None;
                self.start_import(glyph_core::ImportInput::Url(link), Some(typed).filter(|n| !n.is_empty()), cx);
            }
        } else {
            let name = if typed.is_empty() {
                tf("library.new_playlist_name", &[("n", &(self.library.playlists.len() + 1))]).to_string()
            } else {
                typed
            };
            match guard(|| self.library.create_playlist(&name).id.clone()) {
                Some(id) => {
                    self.save_library(cx);
                    match d.tracks.len() {
                        0 => self.open_list(ListRef::User(id), cx),
                        1 => self.add_to_playlist(&id, d.tracks[0].clone(), cx),
                        _ => self.add_many_to_playlist(&id, d.tracks, cx),
                    }
                }
                None => self.toast_info(t("status.library_offline")),
            }
        }
        cx.notify();
    }

    // ------------------------------------------------------------ settings
}
