//! "Local files": music folders on this computer, read in place (see
//! `glyph_core::local_library`). The page browses them by folder, artist,
//! album or as one list of songs.

use super::*;
use glyph_core::local_library::LocalAlbum;
use std::rc::Rc;

/// Tabs of the page.
pub const LOCAL_FOLDERS: u8 = 0;
pub const LOCAL_ARTISTS: u8 = 1;
pub const LOCAL_ALBUMS: u8 = 2;
pub const LOCAL_SONGS: u8 = 3;

/// What the "Local files" page shows.
#[derive(Default)]
pub struct LocalView {
    /// [`LOCAL_FOLDERS`], [`LOCAL_ARTISTS`], [`LOCAL_ALBUMS`] or [`LOCAL_SONGS`].
    pub tab: u8,
    /// Open folder on the folders tab; `None` = the music folders.
    pub dir: Option<PathBuf>,
    /// Open artist on the artists tab.
    pub artist: Option<String>,
    /// Open album on the albums tab (title, artist).
    pub album: Option<(String, String)>,
    /// Filter typed into the search field (lowercase).
    pub query: String,
    /// A scan runs.
    pub scanning: bool,
    /// The folders were read in this session.
    pub scanned: bool,
    cache: Option<(String, Rc<LocalPage>)>,
}

/// Content of the page as it is now (built once per change, not per frame).
#[derive(Default)]
pub struct LocalPage {
    /// Subfolders (folders tab) with their song count.
    pub dirs: Vec<(PathBuf, usize)>,
    /// Artists with their song count (artists tab, none open).
    pub artists: Vec<(String, usize)>,
    /// Albums (albums tab, none open).
    pub albums: Vec<LocalAlbum>,
    /// Songs to list.
    pub tracks: Rc<Vec<Track>>,
}

fn matches(t: &Track, q: &str) -> bool {
    q.is_empty()
        || t.title.to_lowercase().contains(q)
        || t.artist.to_lowercase().contains(q)
        || t.album.as_deref().is_some_and(|a| a.to_lowercase().contains(q))
}

impl Glyph {
    /// The music folders as paths.
    pub fn local_folders(&self) -> Vec<PathBuf> {
        self.config.local.folders.iter().map(PathBuf::from).collect()
    }

    /// Opens the "Local files" page; the folders are read again once per
    /// session (new and changed files).
    pub fn open_local(&mut self, cx: &mut Context<Self>) {
        if self.view != View::Local {
            self.push_route();
            self.view = View::Local;
            self.selected_row = None;
            self.ctx_menu = None;
            self.main_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        }
        if !self.local.scanned {
            self.rescan_local(cx);
        }
        cx.notify();
    }

    /// Reads the music folders again.
    pub fn rescan_local(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        if self.local.scanning {
            return;
        }
        self.local.scanning = true;
        self.local.scanned = true;
        let folders = self.local_folders();
        let scanned_folders = folders.clone();
        let quiet = folders.is_empty();
        cx.spawn(async move |this, cx| {
            let res = guard_async(b.scan_local(folders)).await;
            this.update(cx, |s, cx| {
                s.local.scanning = false;
                s.local.cache = None;
                match res {
                    Ok(r) if r.failed > 0 => s.toast_info(tf("local.scan.failed", &[("n", &r.failed)])),
                    Ok(r) if r.read > 0 && !quiet => s.toast(tf("local.scan.done", &[("n", &r.files)]).to_string()),
                    Ok(_) => {}
                    Err(e) => s.toast_info(format!("{e:#}")),
                }
                if s.local_folders() != scanned_folders {
                    s.rescan_local(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Asks for a folder and adds it to the music folders.
    pub fn add_local_folder(&mut self, cx: &mut Context<Self>) {
        self.open_select = None;
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some(t("local.add").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            this.update(cx, |s, cx| {
                let mut added = false;
                for p in paths {
                    let p = p.to_string_lossy().into_owned();
                    if !s.config.local.folders.iter().any(|f| f.eq_ignore_ascii_case(&p)) {
                        s.config.local.folders.push(p);
                        added = true;
                    }
                }
                if added {
                    s.save_config(cx);
                    s.local.dir = None;
                    s.rescan_local(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Removes a music folder (its files stay where they are).
    pub fn remove_local_folder(&mut self, folder: &str, cx: &mut Context<Self>) {
        self.config.local.folders.retain(|f| f != folder);
        if self.local.dir.as_ref().is_some_and(|d| d.starts_with(folder)) {
            self.local.dir = None;
        }
        self.save_config(cx);
        self.rescan_local(cx);
        cx.notify();
    }

    pub fn set_local_tab(&mut self, tab: u8, cx: &mut Context<Self>) {
        self.local.tab = tab;
        self.selected_row = None;
        self.main_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        cx.notify();
    }

    /// Opens a folder (`None`: back to the music folders).
    pub fn open_local_dir(&mut self, dir: Option<PathBuf>, cx: &mut Context<Self>) {
        self.local.dir = dir;
        self.selected_row = None;
        self.main_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        cx.notify();
    }

    pub fn open_local_artist(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        self.local.artist = name;
        self.selected_row = None;
        self.main_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        cx.notify();
    }

    pub fn open_local_album(&mut self, album: Option<(String, String)>, cx: &mut Context<Self>) {
        self.local.album = album;
        self.selected_row = None;
        self.main_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        cx.notify();
    }

    /// One step up inside the page (folder, artist, album); `false` when
    /// there is nothing to go up to.
    pub fn local_up(&mut self, cx: &mut Context<Self>) -> bool {
        let roots = self.local_folders();
        match self.local.tab {
            LOCAL_FOLDERS => {
                let Some(dir) = self.local.dir.clone() else { return false };
                let up = if roots.iter().any(|r| *r == dir) { None } else { dir.parent().map(std::path::Path::to_path_buf) };
                self.open_local_dir(up, cx);
            }
            LOCAL_ARTISTS if self.local.artist.is_some() => self.open_local_artist(None, cx),
            LOCAL_ALBUMS if self.local.album.is_some() => self.open_local_album(None, cx),
            _ => return false,
        }
        true
    }

    /// Plays what the page lists (everything while it lists folders,
    /// artists or albums).
    pub fn play_local(&mut self, shuffle: bool, cx: &mut Context<Self>) {
        let page = self.local_page();
        let Some(lib) = self.backend.as_ref().map(|b| b.local_library()) else { return };
        let q = &self.local.query;
        let tracks: Vec<Track> = match self.local.tab {
            LOCAL_FOLDERS => {
                let tracks = match &self.local.dir {
                    Some(dir) => lib.tracks_below(dir),
                    None => lib.tracks(),
                };
                tracks.into_iter().filter(|t| matches(t, q)).collect()
            }
            LOCAL_ARTISTS if self.local.artist.is_none() => lib.tracks().into_iter()
                .filter(|t| q.is_empty() || t.artist.to_lowercase().contains(q)).collect(),
            LOCAL_ALBUMS if self.local.album.is_none() => page.albums.iter()
                .flat_map(|album| album.tracks.iter().cloned()).collect(),
            _ => (*page.tracks).clone(),
        };
        if tracks.is_empty() {
            return;
        }
        if shuffle {
            self.play_shuffled(tracks, cx)
        } else {
            self.play_list(tracks, 0, cx)
        }
    }

    /// What the page shows now; rebuilt only when the library, the tab,
    /// the open folder / artist / album or the filter changed.
    pub fn local_page(&mut self) -> Rc<LocalPage> {
        let Some(lib) = self.backend.as_ref().map(|b| b.local_library()) else { return Rc::default() };
        let v = &self.local;
        let key = format!("{}|{}|{:?}|{:?}|{:?}|{}|{}", lib.generation(), v.tab, v.dir, v.artist, v.album, v.query, self.config.local.folders.join(";"));
        if let Some((k, page)) = &self.local.cache {
            if *k == key {
                return page.clone();
            }
        }
        let q = v.query.clone();
        let filter = |tracks: Vec<Track>| -> Rc<Vec<Track>> { Rc::new(tracks.into_iter().filter(|t| matches(t, &q)).collect()) };
        let page = match v.tab {
            LOCAL_FOLDERS => match &v.dir {
                // The music folders themselves, with their counts.
                None => LocalPage {
                    dirs: self.local_folders().into_iter().map(|r| {
                        let n = lib.count_in(&r);
                        (r, n)
                    }).collect(),
                    ..Default::default()
                },
                Some(dir) => {
                    let f = lib.folder(dir);
                    // A filter searches everything below the folder.
                    let tracks = if q.is_empty() {
                        Rc::new(f.tracks)
                    } else {
                        filter(lib.tracks_below(dir))
                    };
                    LocalPage { dirs: if q.is_empty() { f.dirs } else { Vec::new() }, tracks, ..Default::default() }
                }
            },
            LOCAL_ARTISTS => match &v.artist {
                Some(a) => LocalPage { tracks: filter(lib.by_artist(a)), ..Default::default() },
                None => LocalPage { artists: lib.artists().into_iter().filter(|(a, _)| q.is_empty() || a.to_lowercase().contains(&q)).collect(), ..Default::default() },
            },
            LOCAL_ALBUMS => {
                let albums = lib.albums();
                match &v.album {
                    Some((title, artist)) => {
                        let tracks = albums.into_iter().find(|al| al.title == *title && al.artist == *artist).map(|al| al.tracks).unwrap_or_default();
                        LocalPage { tracks: filter(tracks), ..Default::default() }
                    }
                    None => LocalPage {
                        albums: albums
                            .into_iter()
                            .filter(|al| q.is_empty() || al.title.to_lowercase().contains(&q) || al.artist.to_lowercase().contains(&q))
                            .collect(),
                        ..Default::default()
                    },
                }
            }
            _ => LocalPage { tracks: filter(lib.tracks()), ..Default::default() },
        };
        let page = Rc::new(page);
        self.local.cache = Some((key, page.clone()));
        page
    }
}
