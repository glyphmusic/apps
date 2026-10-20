use super::*;

impl Glyph {
    pub fn toggle_like(&mut self, t: &Track, cx: &mut Context<Self>) {
        if !self.library_ok {
            self.toast_info(crate::i18n::t("status.library_offline"));
            return;
        }
        let was = self.is_liked(t);
        if !was {
            self.touch_list(&ListRef::Liked, 0.3);
        }
        if guard(|| self.library.toggle_like(t)).is_some() {
            self.save_library(cx);
            // Liked Songs downloads new likes once it was downloaded.
            if !was && self.library.liked_auto_download && !self.is_downloaded(t) && !self.is_downloading(t) {
                self.download_tracks(vec![t.clone()], cx);
            }
        }
        cx.notify();
    }

    /// Playing from an artist page counts towards the artist's sidebar place
    /// (and gives the entry a picture).
    pub(super) fn touch_artist_page(&mut self, track: &Track) {
        if !self.library_ok || self.view != View::Artist {
            return;
        }
        if let Some(name) = self.open_artist.clone() {
            self.library.push_recent_artist_page(&name, Some(track.clone()));
            self.library.record_use(&glyph_core::Library::artist_id(&name), 1.0, glyph_core::paths::now_unix());
        }
    }

    /// Counts an interaction with a list towards its sidebar place
    /// (listening from it counts 1, playing it 0.5, editing it 0.3-0.5).
    pub fn touch_list(&mut self, l: &ListRef, amount: f64) {
        if self.library_ok {
            self.library.record_list_use(&l.key(), amount, glyph_core::paths::now_unix());
        }
    }

    /// Liked Songs keeps downloading new likes (turned on by downloading the
    /// list, off by removing its downloads).
    pub fn set_liked_auto_download(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.library_ok && self.library.liked_auto_download != on {
            self.library.liked_auto_download = on;
            self.save_library(cx);
        }
    }

    pub fn delete_playlist(&mut self, id: &str, cx: &mut Context<Self>) {
        if guard(|| self.library.delete_playlist(id)).is_some() {
            self.save_library(cx);
        }
        if self.open_list == Some(ListRef::User(id.to_string())) {
            self.history.retain(|r| r.list != Some(ListRef::User(id.to_string())));
            let back = self.history.pop().unwrap_or(Route { view: View::Library, list: None, artist: None, profile: None });
            self.restore(back, cx);
        }
        cx.notify();
    }

    /// Adds several songs at once (one toast).
    pub fn add_many_to_playlist(&mut self, id: &str, tracks: Vec<Track>, cx: &mut Context<Self>) {
        self.touch_list(&ListRef::User(id.to_string()), 0.5);
        let mut added = Vec::new();
        let mut name = String::new();
        if let Some(p) = self.library.playlists.iter_mut().find(|p| p.id == id) {
            name = p.name.clone();
            for t in tracks {
                if !p.tracks.iter().any(|x| x.id == t.id && x.source == t.source) {
                    p.tracks.push(t.clone());
                    added.push(t);
                }
            }
        }
        if !added.is_empty() {
            self.save_library(cx);
            self.toast(tf("toast.added_n_to_playlist", &[("n", &added.len()), ("name", &name)]));
            self.auto_download_added(id, added, cx);
        }
        cx.notify();
    }

    /// Music files / folders dropped onto playlist `id`: imported as local
    /// songs (tags, embedded cover) and added at the end.
    pub fn import_files_to_playlist(&mut self, id: String, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else {
            self.toast_info(t("toast.dl_unready"));
            cx.notify();
            return;
        };
        // Folders are walked off the UI thread; plain files are checked here.
        if !paths.iter().any(|p| p.is_dir() || glyph_core::import::local::is_audio(p)) {
            self.toast_info(t("toast.import_none"));
            cx.notify();
            return;
        }
        self.toast(t("toast.import_started"));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = b.import_local(paths).await;
            this.update(cx, |s, cx| {
                match res {
                    Ok(r) => {
                        if r.failed > 0 {
                            s.toast_info(tf("toast.import_failed", &[("n", &r.failed)]));
                        }
                        let before = s.library.playlist(&id).map(|p| p.tracks.len()).unwrap_or(0);
                        let n = r.tracks.len();
                        s.add_many_to_playlist(&id, r.tracks, cx);
                        let after = s.library.playlist(&id).map(|p| p.tracks.len()).unwrap_or(0);
                        if n == 0 && r.failed == 0 {
                            s.toast_info(t("toast.import_none"));
                        } else if n > 0 && after == before {
                            s.toast_info(t("toast.import_already"));
                        }
                    }
                    Err(e) => s.toast_info(tf("toast.import_error", &[("e", &format!("{e:#}"))])),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn add_to_playlist(&mut self, id: &str, t: Track, cx: &mut Context<Self>) {
        if let Some(p) = self.library.playlists.iter_mut().find(|p| p.id == id) {
            if !p.tracks.iter().any(|x| x.id == t.id && x.source == t.source) {
                p.tracks.push(t.clone());
                let name = p.name.clone();
                self.touch_list(&ListRef::User(id.to_string()), 0.5);
                self.save_library(cx);
                self.toast(tf("toast.added_to_playlist", &[("name", &name)]));
                self.auto_download_added(id, vec![t], cx);
            }
        }
        cx.notify();
    }

    pub fn is_liked_auto_download(&self) -> bool {
        self.library_ok && self.library.liked_auto_download
    }

    pub fn is_auto_download(&self, id: &str) -> bool {
        self.library_ok && self.library.playlist(id).is_some_and(|p| p.auto_download)
    }

    /// Turns "download new songs automatically" on or off for a playlist.
    /// Songs already in it stay as they are ("Download all" covers those).
    pub fn toggle_auto_download(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.library_ok {
            self.toast_info(t("status.library_offline"));
            return;
        }
        let Some(p) = self.library.playlist_mut(id) else { return };
        p.auto_download = !p.auto_download;
        let on = p.auto_download;
        self.save_library(cx);
        self.toast_info(t(if on { "toast.auto_dl_on" } else { "toast.auto_dl_off" }));
        cx.notify();
    }

    /// Songs just added to playlist `id` (here or from another device):
    /// downloads them if the playlist downloads new songs automatically.
    pub(crate) fn auto_download_added(&mut self, id: &str, added: Vec<Track>, cx: &mut Context<Self>) {
        if added.is_empty() || !self.is_auto_download(id) {
            return;
        }
        self.download_tracks_with(added, true, cx);
    }

    pub(super) fn copy_glyph_link(&mut self, title: String, tracks: Vec<Track>, kind: &'static str, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        cx.spawn(async move |this, cx| {
            let text = b.share_text(&title, &tracks, kind, None).await;
            this.update(cx, |s, cx| {
                if glyph_core::share::link_id(&text).is_some() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                    s.toast_sub(t("toast.share_link_copied"), title);
                } else {
                    s.toast_info(t("toast.share_link_failed"));
                }
                cx.notify();
            }).ok();
        }).detach();
    }

    pub fn share(&mut self, t: &Track, cx: &mut Context<Self>) {
        if self.backend.as_ref().is_some_and(|b| b.account().is_some()) {
            self.copy_glyph_link(t.title.clone(), vec![t.clone()], "track", cx);
            return;
        }
        match &t.permalink {
            Some(url) => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(url.clone()));
                self.toast_sub(crate::i18n::t("toast.link_copied_plain"), t.title.clone());
            }
            None => self.toast_info(crate::i18n::t("toast.no_link")),
        }
    }
}
