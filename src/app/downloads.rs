use super::*;

impl Glyph {
    pub fn is_downloaded(&self, t: &Track) -> bool {
        self.backend.as_ref().is_some_and(|b| b.is_downloaded(t))
    }

    pub fn is_downloading(&self, t: &Track) -> bool {
        !self.downloading.is_empty() && self.downloading.contains(&track_key(t))
    }

    /// Download badge state of a song (ring while busy, check when saved).
    pub fn download_view(&self, t: &Track) -> crate::ui::widgets::DlView {
        use crate::ui::widgets::{DlView, DL_DONE_ANIM_S};
        if self.is_downloading(t) {
            // Waiting for its turn: empty ring; running with unknown size: spinner.
            return match self.backend.as_ref().and_then(|b| b.download_progress(t)) {
                None => DlView::Busy(Some(0.0)),
                Some(f) => DlView::Busy(f),
            };
        }
        if !self.is_downloaded(t) {
            return DlView::None;
        }
        let age = if self.download_done.is_empty() {
            None
        } else {
            self.download_done.get(&track_key(t)).map(|at| at.elapsed().as_secs_f32()).filter(|a| *a < DL_DONE_ANIM_S)
        };
        DlView::Done(age)
    }

    /// One badge for several songs: the share already fetched while any of
    /// them downloads, the check once all are saved.
    pub fn downloads_view(&self, tracks: &[Track]) -> crate::ui::widgets::DlView {
        use crate::ui::widgets::DlView;
        if tracks.is_empty() {
            return DlView::None;
        }
        let views: Vec<DlView> = tracks.iter().map(|t| self.download_view(t)).collect();
        if views.iter().any(|v| matches!(v, DlView::Busy(_))) {
            let sum: f32 = views
                .iter()
                .map(|v| match v {
                    DlView::Done(_) => 1.0,
                    DlView::Busy(f) => f.unwrap_or(0.0),
                    DlView::None => 0.0,
                })
                .sum();
            return DlView::Busy(Some(sum / views.len() as f32));
        }
        if views.iter().all(|v| matches!(v, DlView::Done(_))) {
            // Animate when the last one just finished.
            let age = views.iter().filter_map(|v| if let DlView::Done(a) = v { *a } else { None }).reduce(f32::min);
            return DlView::Done(age);
        }
        DlView::None
    }

    /// Tooltip of a list's download button: how many of its songs are saved.
    pub fn download_tip(&self, tracks: &[Track]) -> SharedString {
        let total = tracks.len();
        if total == 0 {
            return t("tip.download");
        }
        let saved = tracks.iter().filter(|t| self.is_downloaded(t)).count();
        if tracks.iter().any(|t| self.is_downloading(t)) {
            return tf("tip.dl_busy", &[("done", &saved), ("total", &total)]);
        }
        if saved == 0 {
            t("tip.download")
        } else {
            tf("tip.dl_saved", &[("done", &saved), ("total", &total)])
        }
    }

    /// Tooltip of one song's download badge: the phase and, while bytes flow,
    /// how far it is. `None` for songs that are neither saved nor downloading
    /// (their row shows no badge).
    pub fn download_tip_one(&self, track: &Track) -> Option<SharedString> {
        if self.is_downloading(track) {
            return Some(match self.backend.as_ref().and_then(|b| b.download_progress(track)) {
                Some(Some(f)) if f < 0.995 => {
                    let pct = (f.clamp(0.0, 1.0) * 100.0).round() as i32;
                    tf("tip.dl_song_busy", &[("pct", &pct)])
                }
                Some(Some(_)) => t("tip.dl_song_finish"),
                _ => t("tip.dl_song_prepare"),
            });
        }
        self.is_downloaded(track).then(|| t("tip.dl_song_done"))
    }

    /// Downloads songs through the shared bounded queue; songs no source offers for
    /// download are skipped and counted in the closing toast.
    pub fn download_tracks(&mut self, tracks: Vec<Track>, cx: &mut Context<Self>) {
        self.download_tracks_with(tracks, false, cx);
    }

    /// `auto`: started by a playlist that downloads new songs by itself, so
    /// it starts silently and only reports what was saved or failed.
    pub(super) fn download_tracks_with(&mut self, tracks: Vec<Track>, auto: bool, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else {
            if !auto {
                self.toast_info(t("toast.dl_unready"));
            }
            cx.notify();
            return;
        };
        let mut seen = HashSet::new();
        let todo: Vec<Track> = tracks.into_iter().filter(|tr| !self.is_downloaded(tr) && !self.is_downloading(tr) && seen.insert(track_key(tr))).collect();
        if todo.is_empty() {
            if !auto {
                self.toast_info(t("toast.dl_already"));
            }
            cx.notify();
            return;
        }
        for tr in &todo {
            self.downloading.insert(track_key(tr));
        }
        let total = todo.len();
        let first_title = todo[0].title.clone();
        if !auto {
            self.toast(if total == 1 {
                tf("toast.dl_started_one", &[("title", &first_title)]).to_string()
            } else {
                tf("toast.dl_started", &[("n", &total)]).to_string()
            });
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (mut ok, mut missing, mut failed) = (0usize, 0usize, 0usize);
            use futures::StreamExt;
            let mut pending = futures::stream::iter(todo).map(|tr| {
                let b = b.clone();
                async move { let result = b.download(&tr).await; (tr, result) }
            }).buffer_unordered(10);
            while let Some((tr, result)) = pending.next().await {
                let saved = match result {
                    Ok(Some(_)) => {
                        ok += 1;
                        true
                    }
                    Ok(None) => {
                        missing += 1;
                        false
                    }
                    Err(e) => {
                        tracing::warn!(title = %tr.title, "download failed: {e:#}");
                        failed += 1;
                        false
                    }
                };
                let k = track_key(&tr);
                this.update(cx, |s, cx| {
                    s.downloading.remove(&k);
                    if saved {
                        s.download_done.retain(|_, at| at.elapsed() < Duration::from_secs(5));
                        s.download_done.insert(k, Instant::now());
                    }
                    cx.notify();
                })
                .ok();
            }
            // Songs no source offers for download are expected in an
            // automatic run; it says nothing unless something happened.
            if auto && ok + failed == 0 {
                return;
            }
            let msg = if ok + failed == 0 {
                t("toast.dl_none").to_string()
            } else if total == 1 && ok == 1 {
                tf("toast.dl_done_one", &[("title", &first_title)]).to_string()
            } else {
                let mut parts = Vec::new();
                if ok > 0 {
                    parts.push(tf("toast.dl_done", &[("n", &ok)]).to_string());
                }
                if missing > 0 {
                    parts.push(tf("toast.dl_unavailable", &[("n", &missing)]).to_string());
                }
                if failed > 0 {
                    parts.push(tf("toast.dl_failed", &[("n", &failed)]).to_string());
                }
                parts.join(" · ")
            };
            this.update(cx, |s, cx| {
                s.toast(msg);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Deletes the downloaded files of `tracks`.
    pub fn remove_downloads(&mut self, tracks: Vec<Track>, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let n = tracks.iter().filter(|tr| b.remove_download(tr)).count();
        if n > 0 {
            self.toast(if n == 1 { t("toast.dl_removed_one").to_string() } else { tf("toast.dl_removed", &[("n", &n)]).to_string() });
        }
        cx.notify();
    }

    pub fn is_following(&self, name: &str) -> bool {
        self.library_ok && self.library.is_following(name)
    }

    pub fn toggle_follow(&mut self, name: &str, track: Option<Track>, cx: &mut Context<Self>) {
        if !self.library_ok {
            self.toast_info(t("status.library_offline"));
            return;
        }
        if let Some(now) = guard(|| self.library.toggle_follow(name, track)) {
            self.save_library(cx);
            self.toast(tf(if now { "toast.following" } else { "toast.unfollowed" }, &[("name", &name)]));
        }
        cx.notify();
    }

    pub fn is_saved(&self, l: &ListRef) -> bool {
        self.library_ok && self.library.is_saved(&l.key())
    }

    /// Heart on an album: save it to / remove it from the library.
    pub fn toggle_saved(&mut self, l: &ListRef, cx: &mut Context<Self>) {
        if self.open_list.as_ref().is_some_and(|open| open.same(l)) {
            if let Some((code, link, author)) = self.shared_preview.take() {
                self.import_code(&code, Some(link), author, cx);
                return;
            }
        }
        if !self.library_ok {
            self.toast_info(t("status.library_offline"));
            return;
        }
        if let Some(saved) = guard(|| self.library.toggle_saved(l.key())) {
            self.save_library(cx);
            self.toast(if saved { t("toast.saved_to_library") } else { t("toast.removed_from_library") });
        }
        cx.notify();
    }
}
