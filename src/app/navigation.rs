use super::*;

impl Glyph {
    /// Recently opened lists that still exist (sidebar).
    /// Ctrl+number: 1 Home, 2 Search, 3 Library, 4 Now Playing, 5..9 the
    /// lists in the sidebar from the top, 0 Settings.
    pub fn nav_slot(&mut self, n: usize, w: &mut Window, cx: &mut Context<Self>) {
        let view = match n {
            0 => Some(View::Settings),
            1 => Some(View::Home),
            2 => Some(View::Search),
            3 => Some(View::Library),
            4 => Some(View::NowPlaying),
            _ => None,
        };
        match view {
            Some(v) => self.navigate(v, w, cx),
            None => {
                match self.sidebar_items(5).into_iter().nth(n - 5) {
                    // Jumping by shortcut leaves the sidebar order as it is.
                    Some(SideItem::List(l)) => {
                        self.nav_quiet = true;
                        self.open_list(l, cx);
                        self.nav_quiet = false;
                    }
                    Some(SideItem::Artist { name, .. }) => {
                        self.nav_quiet = true;
                        self.open_artist(&name, cx);
                        self.nav_quiet = false;
                    }
                    None => {}
                }
            }
        }
    }

    /// Track list of the currently active view for keyboard navigation.
    pub fn active_tracks(&self) -> Vec<Track> {
        match self.view {
            View::List => self.open_list.as_ref().map(|l| self.list_tracks(l)).unwrap_or_default(),
            View::Search => self.search.results.clone(),
            View::Section => self.section.as_ref().map(|(_, t)| t.clone()).unwrap_or_default(),
            View::Artist => self
                .open_artist
                .as_ref()
                .and_then(|n| self.artists.get(n))
                .map(|p| {
                    let lim = if p.show_all { p.tracks.len() } else { p.tracks.len().min(5) };
                    p.tracks.iter().take(lim).cloned().collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// Moves the keyboard row selection up (`delta < 0`) or down (`delta > 0`).
    pub fn select_row_step(&mut self, delta: isize, window: &Window, cx: &mut Context<Self>) {
        if self.settings_open || self.name_dialog.is_some() || self.ctx_menu.is_some() {
            return;
        }
        let n = self.active_tracks().len();
        if n == 0 {
            self.selected_row = None;
            return;
        }
        let next = match self.selected_row {
            Some(cur) => cur.saturating_add_signed(delta).min(n - 1),
            None => {
                if delta >= 0 {
                    0
                } else {
                    n - 1
                }
            }
        };
        self.selected_row = Some(next);
        let row_layout = match self.view {
            View::List => Some(("list-row", 56.0)),
            View::Search => Some(("search-row", 58.0)),
            View::Section => Some(("section-row", 58.0)),
            _ => None,
        };
        if let Some((key, pitch)) = row_layout {
            crate::ui::widgets::scroll_row_into_view(window, key, next, pitch, &self.main_scroll);
        }
        cx.notify();
    }

    /// Plays the active track list starting at the keyboard-selected row.
    pub fn activate_selected_row(&mut self, cx: &mut Context<Self>) {
        if self.settings_open || self.name_dialog.is_some() || self.ctx_menu.is_some() {
            return;
        }
        let Some(i) = self.selected_row else { return };
        let tracks = self.active_tracks();
        if i < tracks.len() {
            self.play_list(tracks, i, cx);
        }
    }

    /// Removes the keyboard-selected song from the open list (Playlist, Liked, Recent, Downloads).
    pub fn remove_selected_row(&mut self, cx: &mut Context<Self>) {
        if self.settings_open || self.name_dialog.is_some() || self.ctx_menu.is_some() {
            return;
        }
        if self.view != View::List {
            return;
        }
        let (Some(l), Some(i)) = (self.open_list.clone(), self.selected_row) else { return };
        let len = self.list_tracks(&l).len();
        if i < len {
            self.remove_from_list(&l, i, cx);
            let new_len = self.list_tracks(&l).len();
            self.selected_row = if new_len == 0 { None } else { Some(i.min(new_len - 1)) };
            cx.notify();
        }
    }

    /// Alt+1 / Alt+2: the current position becomes the loop start / end.
    pub fn set_loop_point(&mut self, start: bool, cx: &mut Context<Self>) {
        if self.current.is_none() {
            return;
        }
        // Mouse over the seek bar: the point goes where the mouse is.
        let pos = match (crate::ui::widgets::bar_hover("seek"), self.duration()) {
            (Some(f), Some(d)) if !d.is_zero() => d.mul_f32(f),
            _ => self.position(),
        };
        self.loop_song = self.current.as_ref().map(track_key);
        let (a, b) = &mut self.loop_ab;
        if start {
            *a = Some(pos);
            if b.is_some_and(|b| b <= pos) {
                *b = None;
            }
        } else if a.map_or(true, |a| pos > a) {
            *b = Some(pos);
            if a.is_none() {
                *a = Some(Duration::ZERO);
            }
        }
        let (a, b) = self.loop_ab;
        let f = |d: Option<Duration>| d.map(crate::ui::widgets::fmt_time).unwrap_or_else(|| "--:--".into());
        self.toast_info(format!("[ LOOP {} -> {} ]", f(a), f(b)));
        cx.notify();
    }

    /// A loop end dragged on the seek bar to `frac` of the song; the ends
    /// keep at least half a second between them.
    pub fn drag_loop_point(&mut self, start: bool, frac: f32, cx: &mut Context<Self>) {
        let Some(d) = self.duration().filter(|d| !d.is_zero()) else { return };
        let pos = d.mul_f32(frac.clamp(0.0, 1.0));
        let min_gap = Duration::from_millis(500);
        let (a, b) = &mut self.loop_ab;
        if start {
            let max = b.map_or(d, |b| b.saturating_sub(min_gap));
            *a = Some(pos.min(max));
        } else {
            let min = a.map_or(Duration::ZERO, |a| a + min_gap);
            *b = Some(pos.max(min).min(d));
        }
        cx.notify();
    }

    pub fn clear_loop(&mut self, cx: &mut Context<Self>) {
        if self.loop_ab != (None, None) {
            self.loop_ab = (None, None);
            self.toast_info("[ LOOP OFF ]");
            cx.notify();
        }
    }

    /// Sidebar entries: every pinned list first, then up to `max` recently
    /// used lists and artist pages (most used first). Hidden ones are left out.
    pub fn sidebar_items(&self, max: usize) -> Vec<SideItem> {
        let exists = |k: &&glyph_core::ListKey| match k {
            glyph_core::ListKey::Playlist { id } => self.library.playlist(id).is_some(),
            _ => true,
        };
        let shown = |id: &str| !self.library.is_sidebar_hidden(id);
        let pinned: Vec<_> = self.library.pinned.iter().filter(exists).collect();
        // Pinned lists count towards `max`: pinning a recent list moves it
        // up instead of letting another list in at the bottom.
        let room = max.saturating_sub(pinned.len());
        // Most used first (listening from a list counts most), ties by recency.
        let now = glyph_core::paths::now_unix();
        let mut recent: Vec<(f64, usize, SideItem)> = Vec::new();
        for (i, k) in self.library.recent_lists.iter().filter(exists).filter(|k| !self.library.pinned.contains(k)).enumerate() {
            if shown(&glyph_core::Library::list_id(k)) {
                recent.push((self.library.list_weight(k, now), i, SideItem::List(ListRef::from_key(k))));
            }
        }
        for (i, a) in self.library.recent_artists.iter().enumerate() {
            let id = glyph_core::Library::artist_id(&a.name);
            if shown(&id) {
                recent.push((self.library.use_weight(&id, now), i, SideItem::Artist { name: a.name.clone(), track: a.track.clone() }));
            }
        }
        recent.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        pinned
            .into_iter()
            .map(|k| SideItem::List(ListRef::from_key(k)))
            .chain(recent.into_iter().take(room).map(|(_, _, it)| it))
            .collect()
    }

    /// Right-click on a sidebar entry: its usual menu plus "Hide from sidebar".
    pub fn open_sidebar_ctx(&mut self, item: SideItem, pos: gpui::Point<gpui::Pixels>, cx: &mut Context<Self>) {
        let id = item.id();
        match item {
            SideItem::List(l) => {
                self.open_list_ctx(l.clone(), pos, cx);
                if self.ctx_menu.is_none() {
                    // An empty list has no menu of its own: just the hide entry.
                    let blank = Track {
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
                    };
                    self.ctx_menu = Some(CtxMenu { track: blank, pos, from: None, playlists: false, list: Some(l), artist: None, sidebar: None });
                    self.ctx_opened = Instant::now();
                    self.ctx_closing = None;
                }
            }
            SideItem::Artist { name, .. } => self.open_artist_ctx(&name, pos, cx),
        }
        if let Some(m) = self.ctx_menu.as_mut() {
            m.sidebar = Some(id);
        }
        cx.notify();
    }

    /// Hides a sidebar entry (or shows it again).
    pub fn toggle_sidebar_hidden(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.library_ok {
            return;
        }
        if guard(|| self.library.toggle_sidebar_hidden(id)) == Some(true) {
            self.toast_info(t("toast.sidebar_hidden"));
        }
        self.save_library(cx);
        cx.notify();
    }

    pub fn is_pinned(&self, l: &ListRef) -> bool {
        self.library.pinned.contains(&l.key())
    }

    /// Pins a list to the top of the sidebar and the library, or unpins it.
    pub fn toggle_pin(&mut self, l: &ListRef, cx: &mut Context<Self>) {
        let k = l.key();
        if let Some(i) = self.library.pinned.iter().position(|p| *p == k) {
            self.library.pinned.remove(i);
        } else {
            // Pinning brings a hidden list back to the sidebar.
            let id = glyph_core::Library::list_id(&k);
            self.library.sidebar_hidden.retain(|h| *h != id);
            self.library.pinned.push(k);
        }
        self.save_library(cx);
        cx.notify();
    }

    /// Liked songs (co-)credited to `name`.
    pub fn liked_by(&self, name: &str) -> Vec<Track> {
        self.library.liked.iter().filter(|t| by_artist(name, &t.artist)).cloned().collect()
    }

    pub fn list_title(&self, l: &ListRef) -> String {
        match l {
            ListRef::Liked => t("library.liked").to_string(),
            ListRef::Recent => t("library.recent").to_string(),
            ListRef::User(id) => self.library.playlist(id).map(|p| p.name.clone()).unwrap_or_default(),
            ListRef::Album { title, .. } => title.clone(),
            ListRef::ArtistLiked(_) => t("library.liked").to_string(),
            ListRef::Downloads => t("library.downloads").to_string(),
        }
    }
}
