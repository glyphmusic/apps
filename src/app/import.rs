use super::*;

/// A link (or file) import running for the "New playlist" dialog: shown
/// there while it is open, finished with a toast when it was closed.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkImport {
    pub seq: u64,
    /// `spotify/playlist`, `file/csv`, ...
    pub label: String,
    pub done: usize,
    pub total: usize,
    pub error: Option<String>,
}

impl Glyph {
    /// What the "link" field of the playlist dialog points to (`None`: empty
    /// or not a link Glyph knows).
    pub fn dialog_link(&self, cx: &Context<Self>) -> Option<(String, glyph_core::import::RemoteSource)> {
        let v = self.playlist_link.read(cx).value.trim().to_string();
        glyph_core::import::parse_url(&v).map(|s| (v, s))
    }

    /// Opens the playlist dialog with a link already filled in and starts
    /// importing it (a playlist link pasted into the search bar).
    pub fn import_link_in_dialog(&mut self, url: &str, window: Option<&mut Window>, cx: &mut Context<Self>) {
        self.playlist_name.update(cx, |i, cx| i.set_value(String::new(), cx));
        self.playlist_link.update(cx, |i, cx| i.set_value(url.to_string(), cx));
        self.ctx_menu = None;
        self.name_dialog = Some(NameDialog { tracks: Vec::new(), rename: None, eq_preset: false, initial_playlist: None });
        if let Some(w) = window {
            let f = self.playlist_name.read(cx).focus.clone();
            w.focus(&f, cx);
        }
        self.start_import(glyph_core::ImportInput::Url(url.to_string()), None, cx);
    }

    /// Reads a link or file, matches every song and saves the list as a
    /// playlist (`name` overrides the list's own name). A link imported
    /// before only gets its new songs.
    pub fn start_import(&mut self, input: glyph_core::ImportInput, name: Option<String>, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else {
            self.toast_info(t("status.library_offline"));
            cx.notify();
            return;
        };
        if !self.library_ok {
            self.toast_info(t("status.library_offline"));
            cx.notify();
            return;
        }
        if self.link_import.as_ref().is_some_and(|l| l.error.is_none()) {
            self.toast_info(t("import.busy"));
            cx.notify();
            return;
        }
        let label = match &input {
            glyph_core::ImportInput::Url(u) => glyph_core::import::parse_url(u).map(|s| s.label().to_string()).unwrap_or_else(|| "link".into()),
            glyph_core::ImportInput::File(p) => format!("file/{}", p.extension().and_then(|e| e.to_str()).unwrap_or("list").to_lowercase()),
        };
        self.link_import_seq += 1;
        let seq = self.link_import_seq;
        self.link_import = Some(LinkImport { seq, label, done: 0, total: 0, error: None });
        cx.notify();
        let (tx, mut rx) = flume_like::channel::<glyph_core::ImportProgress>();
        cx.spawn(async move |this, cx| {
            while let Some(p) = rx.recv().await {
                let ok = this
                    .update(cx, |s, cx| {
                        if let Some(l) = s.link_import.as_mut().filter(|l| l.seq == seq) {
                            l.done = p.done;
                            l.total = p.total;
                            cx.notify();
                        }
                    })
                    .is_ok();
                if !ok {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let r = guard_async(b.import(input, tx)).await;
            this.update(cx, |s, cx| s.finish_import(seq, r, name, cx)).ok();
        })
        .detach();
    }

    fn finish_import(&mut self, seq: u64, r: Result<glyph_core::ImportResult, String>, name: Option<String>, cx: &mut Context<Self>) {
        if self.link_import.as_ref().map(|l| l.seq) != Some(seq) {
            return;
        }
        let dialog_open = self.name_dialog.as_ref().is_some_and(|d| d.rename.is_none() && !d.eq_preset);
        match r {
            Ok(res) if res.ordered.is_empty() => self.import_failed(t("import.empty").to_string(), dialog_open),
            Ok(res) => {
                self.link_import = None;
                let list_name = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| res.name.clone());
                let summary = glyph_core::import::summary(&res);
                let saved = guard(|| {
                    let (p, added) = self.library.save_import(&list_name, res.origin.as_deref(), res.ordered.clone());
                    (p.id.clone(), p.name.clone(), added)
                });
                match saved {
                    Some((id, pname, added)) => {
                        self.save_library(cx);
                        if dialog_open {
                            self.name_dialog = None;
                            self.playlist_link.update(cx, |i, cx| i.set_value(String::new(), cx));
                        }
                        if added == 0 {
                            self.toast_sub(t("import.nothing_new"), pname);
                        } else {
                            self.toast_sub(tf("import.done", &[("n", &added)]), format!("{pname} · {summary}"));
                        }
                        self.open_list(ListRef::User(id), cx);
                    }
                    None => self.toast_info(t("status.library_offline")),
                }
            }
            Err(e) => self.import_failed(e, dialog_open),
        }
        cx.notify();
    }

    /// Shows `msg` under the dialog's link field.
    pub fn link_import_error(&mut self, msg: String) {
        self.link_import_seq += 1;
        self.link_import = Some(LinkImport { seq: self.link_import_seq, label: String::new(), done: 0, total: 0, error: Some(msg) });
    }

    fn import_failed(&mut self, msg: String, dialog_open: bool) {
        if dialog_open {
            if let Some(l) = self.link_import.as_mut() {
                l.error = Some(msg);
            }
        } else {
            self.link_import = None;
            self.toast_info(tf("import.failed", &[("e", &msg)]));
        }
    }

    /// Picks a playlist file (CSV, Spotify JSON export, M3U, XSPF) and imports it.
    pub fn import_playlist_file(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t("import.file.pick").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            this.update(cx, |s, cx| {
                let name = s.name_dialog.as_ref().map(|_| s.playlist_name.read(cx).value.trim().to_string()).filter(|n| !n.is_empty());
                s.start_import(glyph_core::ImportInput::File(path), name, cx);
            })
            .ok();
        })
        .detach();
    }
}
