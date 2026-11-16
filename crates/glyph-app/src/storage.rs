//! Downloads, storage, local music and playlist storage (from
//! `src/app/downloads.rs`, `offline.rs`, `local.rs`, `covers.rs`, `lists.rs`,
//! `library.rs` and the storage part of `settings.rs`).
//!
//! The download queue lives in glyph-core (`Backend::download`); here are the
//! batches, the progress events (`glyph://download`) and the automatic
//! downloads (Liked Songs and playlists that download new songs).

use crate::controller::Core;
use futures::StreamExt;
use glyph_core::local_library::LocalAlbum;
use glyph_core::storage::StorageKind;
use glyph_protocol::Track;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

type Res<T = ()> = Result<T, String>;
type C<'a> = &'a Arc<Core>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// A toast with values in it (`Core::toast_key` has none).
fn toast(core: &Core, key: &str, vars: Value, kind: &str) {
    core.emit("toast", json!({ "key": key, "text": key, "vars": vars, "kind": kind }));
}

/// The key the UI uses for a song (`trackKey` in `api/types.ts`).
fn ui_key(t: &Track) -> String {
    format!("{}|{}", t.title.trim().to_lowercase(), t.artist.trim().to_lowercase())
}

/// Called once at start-up from `glyph_app::setup`.
pub fn setup(core: &Arc<Core>) {
    spawn_auto_downloads(core);
    spawn_connectivity(core);
}

// ---------------------------------------------------------------- downloads

/// Songs downloading now, by UI key.
fn active() -> &'static Mutex<HashMap<String, Track>> {
    static A: OnceLock<Mutex<HashMap<String, Track>>> = OnceLock::new();
    A.get_or_init(Default::default)
}

fn active_lock() -> std::sync::MutexGuard<'static, HashMap<String, Track>> {
    active().lock().unwrap_or_else(|e| e.into_inner())
}

fn done_event(core: &Core, key: &str) {
    core.emit("download", json!({ "key": key, "progress": 1.0, "done": true }));
}

/// Sends the progress of the running downloads until none is left.
fn spawn_ticker(core: &Arc<Core>) {
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let core = core.clone();
    crate::rt::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let list: Vec<(String, Track)> = active_lock().iter().map(|(k, t)| (k.clone(), t.clone())).collect();
            if list.is_empty() {
                break;
            }
            if let Some(b) = core.backend() {
                for (k, t) in list {
                    // Not started yet (waiting for its turn): nothing to say.
                    if let Some(p) = b.download_progress(&t) {
                        core.emit("download", json!({ "key": k, "progress": p, "done": false }));
                    }
                }
            }
        }
        RUNNING.store(false, Ordering::SeqCst);
    });
}

/// Downloads songs through the backend's bounded queue; songs no source
/// offers for download are skipped and counted in the closing toast.
/// `auto`: started by a list that downloads new songs by itself; it starts
/// silently and only reports what was saved or failed.
pub fn download_tracks(core: &Arc<Core>, tracks: Vec<Track>, auto: bool) {
    // Songs the UI marked as busy but that will not run: tell it they are done.
    let skip = |tr: &Track| done_event(core, &ui_key(tr));
    if is_offline() {
        if !auto {
            core.toast_key("offline.download_online", "info");
        }
        tracks.iter().for_each(skip);
        return;
    }
    let Some(b) = core.backend() else {
        if !auto {
            core.toast_key("toast.dl_unready", "info");
        }
        tracks.iter().for_each(skip);
        return;
    };
    let mut seen = HashSet::new();
    let mut todo: Vec<Track> = Vec::new();
    for tr in tracks {
        let key = ui_key(&tr);
        if b.is_downloaded(&tr) {
            skip(&tr);
        } else if active_lock().contains_key(&key) || !seen.insert(key) {
            // Running already: its own events go on.
        } else {
            todo.push(tr);
        }
    }
    if todo.is_empty() {
        if !auto {
            core.toast_key("toast.dl_already", "info");
        }
        return;
    }
    {
        let mut a = active_lock();
        for tr in &todo {
            a.insert(ui_key(tr), tr.clone());
        }
    }
    spawn_ticker(core);
    let total = todo.len();
    let first_title = todo[0].title.clone();
    if !auto {
        if total == 1 {
            toast(core, "toast.dl_started_one", json!({ "title": first_title }), "ok");
        } else {
            toast(core, "toast.dl_started", json!({ "n": total }), "ok");
        }
    }
    let core = core.clone();
    crate::rt::spawn(async move {
        let (mut ok, mut missing, mut failed) = (0usize, 0usize, 0usize);
        let mut saved_at: Option<PathBuf> = None;
        let mut pending = futures::stream::iter(todo)
            .map(|tr| {
                let b = b.clone();
                async move {
                    let r = b.download(&tr).await;
                    (tr, r)
                }
            })
            .buffer_unordered(10);
        while let Some((tr, result)) = pending.next().await {
            let saved = match result {
                Ok(Some(path)) => {
                    ok += 1;
                    saved_at = Some(path);
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
            let key = ui_key(&tr);
            active_lock().remove(&key);
            done_event(&core, &key);
            if saved {
                core.emit_library();
            }
        }
        // Songs no source offers for download are expected in an automatic
        // run; it says nothing unless something happened.
        if auto && ok + failed == 0 {
            return;
        }
        // "Show in folder" on the closing toast (desktop, when wanted).
        let reveal = saved_at
            .filter(|_| ok > 0 && !cfg!(any(target_os = "android", target_os = "ios")) && core.st().config.ui.reveal_downloads);
        let action = reveal.map(|p| json!({ "key": "toast.show_folder", "command": "reveal_path", "args": { "path": p.to_string_lossy() } }));
        let mut msg = json!({ "kind": "ok" });
        if ok + failed == 0 {
            msg["key"] = "toast.dl_none".into();
            msg["text"] = "toast.dl_none".into();
            msg["kind"] = "info".into();
        } else if total == 1 && ok == 1 {
            msg["key"] = "toast.dl_done_one".into();
            msg["text"] = "toast.dl_done_one".into();
            msg["vars"] = json!({ "title": first_title });
        } else {
            // One toast with the parts joined by a dot, like the GPUI app.
            let mut parts = Vec::new();
            if ok > 0 {
                parts.push(crate::i18n::trf(&core, "toast.dl_done", &[("n", &ok.to_string())]));
            }
            if missing > 0 {
                parts.push(crate::i18n::trf(&core, "toast.dl_unavailable", &[("n", &missing.to_string())]));
            }
            if failed > 0 {
                parts.push(crate::i18n::trf(&core, "toast.dl_failed", &[("n", &failed.to_string())]));
            }
            msg["text"] = parts.join(" · ").into();
        }
        if let Some(a) = action {
            msg["action"] = a;
        }
        core.emit("toast", msg);
    });
}

pub fn download(core: C, tracks: Vec<Track>) {
    download_tracks(core, tracks, false);
}

/// Deletes the downloaded file of one song (the UI says how many were removed).
pub fn download_remove(core: C, track: Track) {
    if core.backend().is_some_and(|b| b.remove_download(&track)) {
        core.emit_library();
    }
}

/// Shows a file in the file manager (the "Show in folder" button of the
/// download toast).
pub fn reveal_path(path: String) -> Res {
    crate::host::get().reveal_in_dir(Path::new(&path))
}

// ------------------------------------------------- automatic downloads

/// Liked Songs and playlists that download new songs: watches the lists and
/// downloads what was added (here or from another device).
fn spawn_auto_downloads(core: &Arc<Core>) {
    let core = core.clone();
    crate::rt::spawn(async move {
        // Songs of each list at the last look (by core key).
        let mut known: HashMap<String, HashSet<String>> = HashMap::new();
        let mut first = true;
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if core.backend().is_none() {
                continue;
            }
            let mut fresh: Vec<Track> = Vec::new();
            {
                let st = core.st();
                let lib = &st.library;
                let mut now: HashMap<String, HashSet<String>> = HashMap::new();
                let mut scan = |id: &str, on: bool, tracks: &[Track]| {
                    let old = known.get(id);
                    let keys: HashSet<String> = tracks.iter().map(glyph_core::dedupe::track_key).collect();
                    if on && !first {
                        let mut seen = HashSet::new();
                        for t in tracks {
                            let k = glyph_core::dedupe::track_key(t);
                            if old.is_some_and(|o| !o.contains(&k)) && seen.insert(k) {
                                fresh.push(t.clone());
                            }
                        }
                    }
                    now.insert(id.to_string(), keys);
                };
                scan("liked", lib.liked_auto_download, &lib.liked);
                for p in &lib.playlists {
                    scan(&format!("pl:{}", p.id), p.auto_download, &p.tracks);
                }
                known = now;
            }
            first = false;
            if !fresh.is_empty() {
                download_tracks(&core, fresh, true);
            }
        }
    });
}

fn lib<R>(core: &Core, f: impl FnOnce(&mut glyph_core::Library) -> R) -> R {
    let r = f(&mut core.st().library);
    core.library_changed();
    r
}

fn signed_in(core: &Core) -> bool {
    core.backend().is_some_and(|b| b.account().is_some())
}

/// Where a playlist is kept: 0 this device only, 1 account, 2 account + offline.
pub fn playlist_storage(core: C, id: String, mode: u8) {
    if mode > 0 && !signed_in(&core) {
        return;
    }
    let tracks = {
        let mut st = core.st();
        st.library.checkpoint_playlists();
        let Some(p) = st.library.playlist_mut(&id) else { return };
        p.local_only = mode == 0;
        p.auto_download = mode == 2;
        p.tracks.clone()
    };
    core.library_changed();
    if mode == 2 {
        download_tracks(core, tracks, false);
    }
}

/// "Download new songs automatically" of a playlist. Songs already in it
/// stay as they are ("Download all" covers those).
pub fn playlist_auto_download(core: C, id: String, on: bool) {
    let found = {
        let mut st = core.st();
        match st.library.playlist_mut(&id) {
            Some(p) => {
                p.auto_download = on;
                true
            }
            None => false,
        }
    };
    if found {
        core.library_changed();
        core.toast_key(if on { "toast.auto_dl_on" } else { "toast.auto_dl_off" }, "info");
    }
}

/// Liked Songs keeps downloading new likes (on by downloading the list, off
/// by removing its downloads).
pub fn liked_auto_download(core: C, on: bool) {
    let changed = {
        let mut st = core.st();
        let c = st.library.liked_auto_download != on;
        st.library.liked_auto_download = on;
        c
    };
    if changed {
        core.library_changed();
    }
}

pub fn playlist_restore_version(core: C, revision: String) -> bool {
    lib(&core, |l| l.restore_playlist_version(&revision))
}

// ------------------------------------------------------ playlist covers

/// Lets the user pick a picture for playlist `id` (file dialog); it is
/// copied into the data dir as a 512 px JPEG.
pub async fn playlist_pick_cover(core: C<'_>, id: String) -> Res {
    let Some(src) = core.host.pick_file(None, "Images", &["png", "jpg", "jpeg", "webp"]).await? else {
        return Ok(());
    };
    let pid = id.clone();
    let saved = crate::rt::spawn_blocking(move || {
        let img = image::open(&src).map_err(|e| e.to_string())?;
        // Square crop from the centre, 512 px.
        let side = img.width().min(img.height());
        let img = img.crop_imm((img.width() - side) / 2, (img.height() - side) / 2, side, side).thumbnail(512, 512);
        let dir = glyph_core::paths::data_dir().join("playlist-covers");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        // New name per change: the webview caches pictures by address.
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let path = dir.join(format!("{pid}-{stamp}.jpg"));
        img.to_rgb8().save(&path).map_err(|e| e.to_string())?;
        Ok::<_, String>(path)
    })
    .await
    .map_err(err)?;
    match saved {
        Ok(path) => {
            let old = core.st().library.playlist(&id).and_then(|p| p.cover.clone());
            lib(&core, |l| l.set_playlist_cover(&id, Some(path.display().to_string())));
            if let Some(old) = old {
                let _ = std::fs::remove_file(old);
            }
            Ok(())
        }
        Err(e) => {
            toast(&core, "toast.cover_error", json!({ "e": e }), "info");
            Ok(())
        }
    }
}

pub fn playlist_remove_cover(core: C, id: String) {
    let old = core.st().library.playlist(&id).and_then(|p| p.cover.clone());
    lib(&core, |l| l.set_playlist_cover(&id, None));
    if let Some(old) = old {
        let _ = std::fs::remove_file(old);
    }
}

/// Release year and type of an album (`null` when the catalog does not know it).
pub async fn album_meta(core: C<'_>, title: String, artist: String) -> Res<Value> {
    use glyph_core::catalog::AlbumKind;
    let b = core.backend_or_err()?;
    Ok(match b.album_meta(&title, &artist).await.map_err(err)? {
        Some(m) => {
            let kind = match m.kind {
                AlbumKind::Album => "album",
                AlbumKind::Ep => "ep",
                AlbumKind::Single => "single",
                AlbumKind::Compilation => "compilation",
            };
            json!({ "kind": kind, "year": m.year })
        }
        None => Value::Null,
    })
}

// ------------------------------------------------------------- storage

/// Sizes of the storage folders (off the UI thread).
pub async fn storage_stats(core: C<'_>) -> Res<Value> {
    let local_songs = core.backend().map_or(0, |b| b.local_library().len());
    let s = crate::rt::spawn_blocking(glyph_core::storage::stats).await.map_err(err)?;
    Ok(json!({
        "covers": s.covers, "downloads": s.downloads, "lyrics": s.lyrics, "catalog": s.catalog, "streams": s.audio_ranges,
        "data_dir": glyph_core::paths::data_dir().to_string_lossy(),
        "default_download_dir": glyph_core::storage::default_downloads_dir().to_string_lossy(),
        "local_songs": local_songs,
    }))
}

pub async fn storage_clear(core: C<'_>, kind: String) -> Res {
    let k = match kind.as_str() {
        "covers" => StorageKind::Covers,
        "downloads" => StorageKind::Downloads,
        "lyrics" => StorageKind::Lyrics,
        "catalog" => StorageKind::Catalog,
        "streams" => StorageKind::AudioRanges,
        other => return Err(format!("unknown kind {other}")),
    };
    crate::rt::spawn_blocking(move || glyph_core::storage::clear(k)).await.map_err(err)?.map_err(|e| format!("{e:#}"))?;
    if let Some(b) = core.backend() {
        match k {
            StorageKind::Catalog => b.clear_catalog_caches(),
            StorageKind::Downloads => {
                b.reload_downloads();
                core.emit_library();
            }
            _ => {}
        }
    }
    Ok(())
}

/// Moves the downloads to a folder the user picks (`pick`) or back to Glyph's
/// own folder (`dir: null`).
pub async fn storage_set_download_dir(core: C<'_>, pick: Option<bool>, dir: Option<String>) -> Res {
    let target: Option<PathBuf> = if pick.unwrap_or(false) {
        match core.host.pick_folders(None, false).await?.and_then(|mut v| v.pop()) {
            Some(p) => Some(p),
            None => return Ok(()),
        }
    } else {
        dir.map(PathBuf::from)
    };
    let b = core.backend_or_err()?;
    if !active_lock().is_empty() {
        core.toast_key("toast.dl_dir_busy", "info");
        return Ok(());
    }
    core.toast_key("toast.dl_dir_moving", "ok");
    match b.set_download_dir(target.clone()).await {
        Ok(r) => {
            core.st().config.storage.download_dir = target.map(|d| d.to_string_lossy().into_owned());
            core.config_changed();
            if r.failed > 0 {
                toast(&core, "toast.dl_dir_failed", json!({ "n": r.failed }), "info");
            } else {
                toast(&core, "toast.dl_dir_done", json!({ "n": r.moved }), "ok");
            }
            core.emit_library();
            Ok(())
        }
        Err(e) => {
            core.emit("toast", json!({ "text": format!("{e:#}"), "kind": "info" }));
            Ok(())
        }
    }
}

// ----------------------------------------------------------- local music

/// Native folder dialog (several folders allowed); `null` when cancelled.
pub async fn pick_folders(core: C<'_>, prompt: Option<String>) -> Res<Option<Vec<String>>> {
    let picked = core.host.pick_folders(prompt, true).await?;
    Ok(picked.map(|v| v.into_iter().map(|p| p.to_string_lossy().into_owned()).collect()))
}

/// Reads the music folders again.
pub async fn local_scan(core: C<'_>, folders: Vec<String>) -> Res<Value> {
    let b = core.backend_or_err()?;
    let r = b.scan_local(folders.into_iter().map(PathBuf::from).collect()).await.map_err(|e| format!("{e:#}"))?;
    Ok(json!({ "files": r.files, "read": r.read, "failed": r.failed }))
}

fn matches(t: &Track, q: &str) -> bool {
    q.is_empty()
        || t.title.to_lowercase().contains(q)
        || t.artist.to_lowercase().contains(q)
        || t.album.as_deref().is_some_and(|a| a.to_lowercase().contains(q))
}

fn dir_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string_lossy().into_owned())
}

fn filtered(tracks: Vec<Track>, q: &str) -> Vec<Track> {
    tracks.into_iter().filter(|t| matches(t, q)).collect()
}

/// What the "Local files" page shows for the tab / folder / artist / album
/// and the filter (`query`, lower case).
pub async fn local_page(
    core: C<'_>,
    tab: u8,
    dir: Option<String>,
    artist: Option<String>,
    album: Option<(String, String)>,
    query: Option<String>,
) -> Res<Value> {
    let Some(b) = core.backend() else {
        return Ok(json!({ "total": 0, "progress": null, "dirs": [], "artists": [], "albums": [], "tracks": [] }));
    };
    let lib = b.local_library();
    let q = query.unwrap_or_default();
    let roots: Vec<PathBuf> = core.st().config.local.folders.iter().map(PathBuf::from).collect();
    let (mut dirs, mut artists, mut albums, mut tracks) = (Vec::<Value>::new(), Vec::<Value>::new(), Vec::<Value>::new(), Vec::<Track>::new());
    let dir_json = |p: &Path, n: usize| json!({ "path": p.to_string_lossy(), "name": dir_name(p), "count": n });
    match tab {
        0 => match dir {
            // The music folders themselves, with their counts.
            None => dirs = roots.iter().map(|r| dir_json(r, lib.count_in(r))).collect(),
            Some(d) => {
                let d = PathBuf::from(d);
                if q.is_empty() {
                    let f = lib.folder(&d);
                    dirs = f.dirs.iter().map(|(p, n)| dir_json(p, *n)).collect();
                    tracks = f.tracks;
                } else {
                    // A filter searches everything below the folder.
                    tracks = filtered(lib.tracks_below(&d), &q);
                }
            }
        },
        1 => match artist {
            Some(a) => tracks = filtered(lib.by_artist(&a), &q),
            None => {
                artists = lib
                    .artists()
                    .into_iter()
                    .filter(|(a, _)| q.is_empty() || a.to_lowercase().contains(&q))
                    .map(|(name, count)| json!({ "name": name, "count": count }))
                    .collect()
            }
        },
        2 => {
            let all = lib.albums();
            match album {
                Some((title, artist)) => {
                    let t = all.into_iter().find(|al| al.title == title && al.artist == artist).map(|al| al.tracks).unwrap_or_default();
                    tracks = filtered(t, &q);
                }
                None => {
                    albums = all
                        .into_iter()
                        .filter(|al| q.is_empty() || al.title.to_lowercase().contains(&q) || al.artist.to_lowercase().contains(&q))
                        .map(|al: LocalAlbum| json!({ "title": al.title, "artist": al.artist, "count": al.tracks.len(), "cover": al.tracks.first() }))
                        .collect()
                }
            }
        }
        _ => tracks = filtered(lib.tracks(), &q),
    }
    Ok(json!({
        "total": lib.len(),
        "progress": lib.scan_progress().map(|(a, b)| [a, b]),
        "dirs": dirs, "artists": artists, "albums": albums, "tracks": tracks,
    }))
}

/// Plays what the page lists (everything while it lists folders, artists
/// or albums).
pub async fn local_play(
    core: C<'_>,
    tab: u8,
    dir: Option<String>,
    artist: Option<String>,
    album: Option<(String, String)>,
    query: Option<String>,
    shuffle: Option<bool>,
) -> Res {
    let b = core.backend_or_err()?;
    let lib = b.local_library();
    let q = query.unwrap_or_default();
    let tracks: Vec<Track> = match tab {
        0 => match dir {
            // Everything below the folder, like the GPUI page.
            Some(d) => filtered(lib.tracks_below(Path::new(&d)), &q),
            None => filtered(lib.tracks(), &q),
        },
        1 => match artist {
            Some(a) => filtered(lib.by_artist(&a), &q),
            None => lib.tracks().into_iter().filter(|t| q.is_empty() || t.artist.to_lowercase().contains(&q)).collect(),
        },
        2 => match album {
            Some((title, artist)) => filtered(lib.albums().into_iter().find(|al| al.title == title && al.artist == artist).map(|al| al.tracks).unwrap_or_default(), &q),
            None => lib
                .albums()
                .into_iter()
                .filter(|al| q.is_empty() || al.title.to_lowercase().contains(&q) || al.artist.to_lowercase().contains(&q))
                .flat_map(|al| al.tracks)
                .collect(),
        },
        _ => filtered(lib.tracks(), &q),
    };
    if tracks.is_empty() {
        return Ok(());
    }
    let core = core;
    if shuffle.unwrap_or(false) {
        // The whole list dealt shuffled (smart or random, as set), as in GPUI.
        core.play_shuffled(tracks, None, crate::controller::PlayFrom::default());
    } else {
        core.play_list(tracks, 0, None);
    }
    Ok(())
}

// ------------------------------------------------------------- offline

static OFFLINE: AtomicBool = AtomicBool::new(false);
static OFFLINE_SONGS: AtomicUsize = AtomicUsize::new(0);

fn audio() -> &'static RwLock<glyph_core::OfflineAudio> {
    static A: OnceLock<RwLock<glyph_core::OfflineAudio>> = OnceLock::new();
    A.get_or_init(Default::default)
}

/// No internet (only ever true on phones, like the GPUI app).
pub fn is_offline() -> bool {
    match glyph_core::connectivity::platform_online() {
        Some(online) => !online,
        None => OFFLINE.load(Ordering::Relaxed),
    }
}

/// Applies a platform connectivity transition right away instead of waiting
/// up to 15 s for `spawn_connectivity`.
pub fn set_offline_now(core: Option<&Arc<Core>>, online: bool) {
    let was = OFFLINE.swap(!online, Ordering::Relaxed);
    if was != !online {
        if let Some(c) = core {
            c.on_offline_changed(!online);
        }
    }
}

/// Saved (downloaded, imported, local) or cached for offline play (phones).
#[allow(dead_code)]
pub fn available_offline(core: &Core, t: &Track) -> bool {
    core.backend().is_some_and(|b| b.is_downloaded(t)) || audio().read().unwrap_or_else(|e| e.into_inner()).contains(t)
}

/// Every 15 s on phones: online or not, and which cached songs can play offline.
fn spawn_connectivity(core: &Arc<Core>) {
    if !cfg!(any(target_os = "android", target_os = "ios")) {
        return;
    }
    let core = core.clone();
    crate::rt::spawn(async move {
        loop {
            if let Some(b) = core.backend() {
                let found = crate::rt::spawn_blocking(glyph_core::OfflineAudio::scan).await.ok();
                if let Some(a) = found {
                    *audio().write().unwrap_or_else(|e| e.into_inner()) = a;
                    OFFLINE_SONGS.fetch_add(1, Ordering::Relaxed);
                }
                // The latest system callback beats a probe started before a network switch.
                let online = match glyph_core::connectivity::platform_online() {
                    Some(o) => o,
                    None => b.internet_available().await,
                };
                let was = OFFLINE.swap(!online, Ordering::Relaxed);
                if was != !online {
                    core.on_offline_changed(!online);
                }
            }
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
    });
}
