use super::*;

impl Glyph {
    /// Load tile portraits without loading songs, releases or related artists.
    pub fn artist_photo_path(&mut self, name: &str, cx: &mut Context<Self>) -> Option<PathBuf> {
        if let Some(path) = self.artists.get(name).and_then(|p| p.info.as_ref()).and_then(|i| i.image.clone()) {
            return Some(path);
        }
        let key = glyph_core::dedupe::normalize_artist(name);
        match self.artist_photos.get(&key) {
            Some(ArtistPhoto::Ready(path)) => return Some(path.clone()),
            Some(ArtistPhoto::Loading | ArtistPhoto::Missing) => return None,
            Some(ArtistPhoto::RetryAt(at)) if Instant::now() < *at => return None,
            _ => {}
        }
        let Some(b) = self.backend.clone() else { return None };
        if !self.artist_photos.contains_key(&key) && self.artist_photos.len() >= 256 {
            let evict = self.artist_photos.iter().find(|(_, photo)| !matches!(photo, ArtistPhoto::Loading)).map(|(key, _)| key.clone());
            if let Some(evict) = evict { self.artist_photos.remove(&evict); } else { return None; }
        }
        self.artist_photos.insert(key.clone(), ArtistPhoto::Loading);
        let name = name.to_string();
        let lang = self.lang.code();
        cx.spawn(async move |this, cx| {
            let photo = guard_async(b.artist_photo(&name, lang)).await;
            this.update(cx, |s, cx| {
                s.artist_photos.insert(key, match photo {
                    Ok(Some(path)) => ArtistPhoto::Ready(path),
                    Ok(None) => ArtistPhoto::Missing,
                    Err(_) => ArtistPhoto::RetryAt(Instant::now() + Duration::from_secs(30)),
                });
                cx.notify();
            }).ok();
        }).detach();
        None
    }

    /// Opens the artist profile (first artist of a "A, B" credit).
    pub fn open_artist(&mut self, name: &str, cx: &mut Context<Self>) {
        let name = primary_artist(name);
        if name.is_empty() || (self.view == View::Artist && self.open_artist.as_deref() == Some(name.as_str())) {
            return;
        }
        self.push_route();
        self.view = View::Artist;
        self.selected_row = None;
        self.main_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        self.open_artist = Some(name.clone());
        self.open_list = None;
        self.ctx_menu = None;
        // Artist pages show in the sidebar next to the lists.
        if self.library_ok && !self.nav_quiet {
            let track = self.artists.get(&name).and_then(|p| p.tracks.first().cloned());
            self.library.push_recent_artist_page(&name, track);
            self.library.record_use(&glyph_core::Library::artist_id(&name), 0.25, glyph_core::paths::now_unix());
            self.save_library_activity();
        }
        self.load_artist(name, cx);
        cx.notify();
    }

    /// "Show all" of an artist's releases: all albums on their own page.
    pub fn open_artist_releases(&mut self, name: &str, cx: &mut Context<Self>) {
        if name.is_empty() || (self.view == View::Releases && self.open_artist.as_deref() == Some(name)) {
            return;
        }
        self.push_route();
        self.view = View::Releases;
        self.selected_row = None;
        self.main_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        self.open_artist = Some(name.to_string());
        self.open_list = None;
        self.ctx_menu = None;
        self.load_artist(name.to_string(), cx);
        cx.notify();
    }

    /// "Show all" of an artist's similar artists: the full grid on its own page.
    pub fn open_artist_similar(&mut self, name: &str, cx: &mut Context<Self>) {
        if name.is_empty() || (self.view == View::Similar && self.open_artist.as_deref() == Some(name)) {
            return;
        }
        self.push_route();
        self.view = View::Similar;
        self.selected_row = None;
        self.main_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        self.open_artist = Some(name.to_string());
        self.open_list = None;
        self.ctx_menu = None;
        self.load_artist(name.to_string(), cx);
        cx.notify();
    }

    pub(super) fn load_artist(&mut self, name: String, cx: &mut Context<Self>) {
        if self.artists.contains_key(&name) {
            return;
        }
        let Some(b) = self.backend.clone() else { return };
        if self.artists.len() >= 64 {
            if let Some(evict) = self
                .artists
                .iter()
                .find(|(k, p)| Some(*k) != self.open_artist.as_ref() && !p.loading && !p.info_loading)
                .map(|(k, _)| k.clone())
            {
                self.artists.remove(&evict);
            }
        }
        // Songs: a normal search for the name, keeping the artist's own songs.
        let songs = {
            let (b, name) = (b.clone(), name.clone());
            cx.spawn(async move |this, cx| {
                let mut found: Vec<Track> = Vec::new();
                if let Some(mut stream) = guard(|| b.search(&name)) {
                    use futures::StreamExt;
                    while let Some(ev) = stream.next().await {
                        match ev {
                            SearchEvent::Results { tracks, .. } => {
                                let mut changed = false;
                                for tr in tracks {
                                    let dup = found.iter().any(|x| {
                                        glyph_core::dedupe::normalize_title(&x.title)
                                            == glyph_core::dedupe::normalize_title(&tr.title)
                                    });
                                    if by_artist(&name, &tr.artist) && !dup {
                                        found.push(tr);
                                        changed = true;
                                    }
                                }
                                if changed {
                                    let list = found.clone();
                                    let n = name.clone();
                                    this.update(cx, |s, cx| {
                                        if let Some(p) = s.artists.get_mut(&n) {
                                            p.tracks = list;
                                            cx.notify();
                                        }
                                    })
                                    .ok();
                                }
                            }
                            SearchEvent::Done => break,
                            _ => {}
                        }
                    }
                }
                this.update(cx, |s, cx| {
                    if let Some(p) = s.artists.get_mut(&name) {
                        p.loading = false;
                    }
                    cx.notify();
                })
                .ok();
            })
        };
        let stats = {
            let (name, b) = (name.clone(), b.clone());
            cx.spawn(async move |this, cx| {
                let r = guard_async(b.artist_stats(&name)).await;
                this.update(cx, |s, cx| {
                    if let (Some(p), Ok(Some(st))) = (s.artists.get_mut(&name), r) {
                        p.stats = Some(st);
                        cx.notify();
                    }
                })
                .ok();
            })
        };
        let similar = {
            let (name, b) = (name.clone(), b.clone());
            cx.spawn(async move |this, cx| {
                let r = guard_async(b.similar_artists(&name)).await;
                this.update(cx, |s, cx| {
                    if let Some(p) = s.artists.get_mut(&name) {
                        p.similar_loading = false;
                        if let Ok(v) = r {
                            // The artist itself is never its own suggestion.
                            p.similar = v.into_iter().filter(|n| !n.eq_ignore_ascii_case(&name)).collect();
                        }
                    }
                    cx.notify();
                })
                .ok();
            })
        };
        let releases = {
            let (name, b) = (name.clone(), b.clone());
            cx.spawn(async move |this, cx| {
                let r = guard_async(b.artist_releases(&name)).await;
                this.update(cx, |s, cx| {
                    if let (Some(p), Ok(v)) = (s.artists.get_mut(&name), r) {
                        p.releases = v;
                        cx.notify();
                    }
                })
                .ok();
            })
        };
        let info = {
            let (name, lang) = (name.clone(), self.lang.code());
            cx.spawn(async move |this, cx| {
                let r = guard_async(b.artist_info(&name, lang)).await;
                this.update(cx, |s, cx| {
                    if let Some(p) = s.artists.get_mut(&name) {
                        p.info_loading = false;
                        if let Ok(Some(i)) = r {
                            p.info = Some(i);
                        }
                    }
                    cx.notify();
                })
                .ok();
            })
        };
        self.artists.insert(
            name.clone(),
            ArtistPage {
                name,
                tracks: Vec::new(),
                loading: true,
                info: None,
                info_loading: true,
                stats: None,
                similar: Vec::new(),
                similar_loading: true,
                releases: Vec::new(),
                show_all: false,
                about_open: false,
                _tasks: vec![songs, info, stats, similar, releases],
            },
        );
    }

    /// Opens the album of `seed` and fills in its other songs: from the
    /// library if saved, otherwise by searching "artist album" in the background.
    pub fn open_album(&mut self, title: &str, artist: &str, seed: Track, cx: &mut Context<Self>) {
        let artist = primary_artist(artist);
        let probe = ListRef::Album { title: title.to_string(), artist: artist.clone(), tracks: vec![seed] };
        if let Some(saved) = self.library.saved_albums.iter().find(|k| k.same(&probe.key())).cloned() {
            self.open_list(ListRef::from_key(&saved), cx);
            return;
        }
        self.open_list(probe, cx);
        let Some(b) = self.backend.clone() else { return };
        let (title, q) = (title.to_string(), format!("{artist} {title}"));
        self.album_task = Some(cx.spawn(async move |this, cx| {
            let Some(mut stream) = guard(|| b.search(&q)) else { return };
            use futures::StreamExt;
            while let Some(ev) = stream.next().await {
                match ev {
                    SearchEvent::Results { tracks, .. } => {
                        let hits: Vec<Track> = tracks
                            .into_iter()
                            .filter(|t| t.album.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(&title)) && by_artist(&artist, &t.artist))
                            .collect();
                        if hits.is_empty() {
                            continue;
                        }
                        let (t2, a2) = (title.clone(), artist.clone());
                        this.update(cx, |s, cx| {
                            if let Some(ListRef::Album { title, artist, tracks }) = s.open_list.as_mut() {
                                if title.eq_ignore_ascii_case(&t2) && artist.eq_ignore_ascii_case(&a2) {
                                    for h in hits {
                                        let n = glyph_core::dedupe::normalize_title(&h.title);
                                        if !tracks.iter().any(|x| glyph_core::dedupe::normalize_title(&x.title) == n) {
                                            tracks.push(h);
                                        }
                                    }
                                    cx.notify();
                                }
                            }
                        })
                        .ok();
                    }
                    SearchEvent::Done => break,
                    _ => {}
                }
            }
        }));
    }

    /// Opens a release from the discography and loads its songs.
    pub fn open_release(&mut self, r: glyph_core::providers::deezer::Release, cx: &mut Context<Self>) {
        let probe = ListRef::Album { title: r.title.clone(), artist: r.artist.clone(), tracks: Vec::new() };
        if let Some(saved) = self.library.saved_albums.iter().find(|k| k.same(&probe.key())).cloned() {
            self.open_list(ListRef::from_key(&saved), cx);
            return;
        }
        self.album_meta.insert(
            format!("{}|{}", r.title.to_lowercase(), r.artist.to_lowercase()),
            Some(glyph_core::catalog::AlbumMeta { year: r.year, kind: r.kind }),
        );
        self.open_list(probe, cx);
        let Some(b) = self.backend.clone() else { return };
        self.album_task = Some(cx.spawn(async move |this, cx| {
            let Ok(tracks) = guard_async(b.release_tracks(&r)).await else { return };
            this.update(cx, |s, cx| {
                if let Some(ListRef::Album { title, artist, tracks: shown }) = s.open_list.as_mut() {
                    if title.eq_ignore_ascii_case(&r.title) && artist.eq_ignore_ascii_case(&r.artist) {
                        *shown = tracks;
                        cx.notify();
                    }
                }
            })
            .ok();
        }));
    }

    /// Plays the `i`-th recently played song (taskbar jump list / dock menu).
    pub fn play_recent(&mut self, i: usize, cx: &mut Context<Self>) {
        let list: Vec<Track> = self.library.recent_tracks().cloned().collect();
        if i < list.len() {
            self.play_list(list, i, cx);
        }
    }

    /// Plays (or opens) the `i`-th entry of "recently played": a song starts
    /// the list at its song, an artist opens their page.
    pub fn play_recent_item(&mut self, i: usize, cx: &mut Context<Self>) {
        match self.library.recent.get(i).cloned() {
            Some(RecentItem::Artist { name, .. }) => self.open_artist(&name, cx),
            Some(RecentItem::Song { track }) => {
                let list: Vec<Track> = self.library.recent_tracks().cloned().collect();
                let key = crate::app::track_key(&track);
                let song = list.iter().position(|t| crate::app::track_key(t) == key).unwrap_or(0);
                self.play_list(list, song, cx);
            }
            None => {}
        }
    }

    /// "Remove from recently played" for one song (artists are left alone).
    pub fn remove_recent_track(&mut self, track: &Track, cx: &mut Context<Self>) {
        if guard(|| self.library.remove_recent_song(track)).is_none() {
            return;
        }
        self.save_library(cx);
        cx.notify();
    }
}
