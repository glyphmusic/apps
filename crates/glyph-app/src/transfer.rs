//! Importing playlists (link, file, pasted text) and moving a whole library
//! from another service (Settings > Transfer music). From
//! `src/app/import.rs` and `src/ui/migrate.rs`. Long jobs run in the
//! background; the UI polls `migrate_progress` (see migrate-state.svelte.ts).

use crate::controller::Core;
use crate::guard::guard_async;
use glyph_core::flume_like;
use glyph_core::import::accounts::{
    Collection, Kind, LibrarySource, Login, MigrationEvent, MigrationState, Migrated, Overview, Service, Totals,
};
use glyph_core::{Backend, ImportInput, ImportProgress};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

type Res<T = ()> = Result<T, String>;
type C<'a> = &'a Arc<Core>;

/// What a move needs between "overview" and "start".
struct Choice {
    service: Service,
    source: Arc<dyn LibrarySource>,
    overview: Overview,
    /// What was handed over (kept so a stopped run can continue).
    login: Login,
}

/// Where a migrated track collection should be saved.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MigrateTarget {
    Liked,
    Playlist { id: String },
    NewPlaylist { name: Option<String> },
}

/// The move that runs (or ran last).
struct Run {
    #[allow(dead_code)]
    service: Service,
    index: usize,
    lists: usize,
    current: String,
    done: usize,
    total: usize,
    totals: Totals,
    targets: HashMap<String, MigrateTarget>,
    failed: Vec<(String, String)>,
    notes: Vec<String>,
    cancel: Arc<AtomicBool>,
    state: MigrationState,
    finished: bool,
    cancelled: bool,
}

#[derive(Default)]
struct Inner {
    choice: Option<Choice>,
    run: Option<Run>,
}

#[derive(Default)]
pub struct Transfer {
    inner: Mutex<Inner>,
    /// A link or file import is running.
    importing: AtomicBool,
}

impl Transfer {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn running(&self) -> bool {
        self.lock().run.as_ref().is_some_and(|r| !r.finished)
    }
}

/// Called once at start-up from `glyph_app::setup`.
pub fn setup(core: &Arc<Core>) {
    core.set_transfer(Arc::new(Transfer::default()));
}

fn need_transfer(core: &Core) -> Res<Arc<Transfer>> {
    core.transfer().ok_or_else(|| "transfer not ready".to_string())
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// A toast with a key the UI translates, `{vars}` and a second line.
fn toast(core: &Core, key: &str, vars: Value, sub: Option<String>, kind: &str) {
    core.emit("toast", json!({ "key": key, "text": key, "vars": vars, "sub": sub, "kind": kind }));
}

// ---- links and files ----

/// What a finished import saved.
struct Saved {
    id: String,
    name: String,
    added: usize,
    summary: String,
}

/// Clears the "importing" flag when the import ends, however it ends.
struct Busy<'a>(&'a AtomicBool);
impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn save_tracks(
    core: &Arc<Core>,
    list_name: &str,
    origin: Option<&str>,
    tracks: Vec<glyph_protocol::Track>,
    playlist_id: Option<&str>,
    summary: String,
) -> Saved {
    let (id, pname, added) = {
        let mut st = core.st();
        if let Some((p, added)) = playlist_id.and_then(|pid| st.library.add_imported_to_playlist(pid, tracks.clone())) {
            (p.id.clone(), p.name.clone(), added)
        } else {
            let (p, added) = st.library.save_import(list_name, origin, tracks);
            (p.id.clone(), p.name.clone(), added)
        }
    };
    core.library_changed();
    Saved { id, name: pname, added, summary }
}

/// Reads a link or file, matches every song and saves the list into
/// `playlist_id` (if given) or as a playlist (`name` overrides the list's own
/// name). A link imported before only gets its new songs.
async fn import_and_save(
    core: &Arc<Core>,
    tr: &Transfer,
    input: ImportInput,
    name: Option<String>,
    playlist_id: Option<&str>,
) -> Res<Saved> {
    // Errors made here are i18n keys; `say` turns them into words.
    let b = core.backend_or_err().map_err(|_| "status.library_offline".to_string())?;
    if tr.importing.swap(true, Ordering::AcqRel) {
        return Err("import.busy".into());
    }
    let _busy = Busy(&tr.importing);
    let (tx, mut rx) = flume_like::channel::<ImportProgress>();
    // The dialog shows "done / total" while the list is read.
    let c2 = core.clone();
    let drain = crate::rt::spawn(async move {
        while let Some(p) = rx.recv().await {
            c2.emit("import_progress", json!({ "done": p.done, "total": p.total }));
        }
    });
    let r = guard_async(b.import(input, tx)).await;
    drain.abort();
    core.emit("import_progress", json!({ "done": 0, "total": 0 }));
    let res = r?;
    if res.ordered.is_empty() {
        return Err("import.empty".into());
    }
    let list_name = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| res.name.clone());
    let summary = glyph_core::import::summary(&res);
    Ok(save_tracks(core, &list_name, res.origin.as_deref(), res.ordered, playlist_id, summary))
}

/// Imports copied song links (e.g. Ctrl+A, Ctrl+C in Spotify) into `playlist_id`
/// or a new playlist.
async fn import_song_links(
    core: &Arc<Core>,
    tr: &Transfer,
    text: &str,
    name: Option<String>,
    playlist_id: Option<&str>,
) -> Res<Saved> {
    let b = core.backend_or_err().map_err(|_| "status.library_offline".to_string())?;
    if tr.importing.swap(true, Ordering::AcqRel) {
        return Err("import.busy".into());
    }
    let _busy = Busy(&tr.importing);
    let tracks = guard_async(b.songs_from_links(text)).await?;
    if tracks.is_empty() {
        return Err("import.empty".into());
    }
    let list_name = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Imported playlist".to_string());
    let summary = format!("{} songs", tracks.len());
    Ok(save_tracks(core, &list_name, None, tracks, playlist_id, summary))
}

/// An import error as words: ours are i18n keys, the core's are already text.
fn say(core: &Core, e: &str) -> String {
    if e.starts_with("import.") || e.starts_with("status.") || e.starts_with("migrate.") {
        crate::i18n::tr(core, e)
    } else {
        e.to_string()
    }
}

/// The import failed: a toast.
fn failed_toast(core: &Core, e: &str) {
    let text = say(core, e);
    toast(core, "import.failed", json!({ "e": text }), None, "info");
}

fn saved_toast(core: &Core, s: &Saved) {
    if s.added == 0 {
        toast(core, "import.nothing_new", Value::Null, Some(s.name.clone()), "info");
    } else {
        toast(core, "import.done", json!({ "n": s.added }), Some(format!("{} · {}", s.name, s.summary)), "ok");
    }
}

/// What the link is (`spotify/playlist`), or nothing for text Glyph can't import.
pub fn import_parse_url(url: String) -> Option<String> {
    glyph_core::import::parse_url(url.trim()).map(|s| s.label().to_string())
}

/// Imports a link (or copied song links) into `playlist_id` or a new
/// playlist. The UI shows `error` under the field.
pub async fn import_link(core: C<'_>, url: String, name: Option<String>, playlist_id: Option<String>) -> Res<Value> {
    let tr = need_transfer(core)?;
    let core = core.clone();
    let trimmed = url.trim().to_string();
    let (songs, lists) = Backend::pasted_links(&trimmed);
    let result = if songs > 0 && lists == 0 {
        import_song_links(&core, &tr, &trimmed, name, playlist_id.as_deref()).await
    } else {
        import_and_save(&core, &tr, ImportInput::Url(trimmed), name, playlist_id.as_deref()).await
    };
    match result {
        Ok(s) => {
            saved_toast(&core, &s);
            Ok(json!({ "error": null, "playlist": s.id }))
        }
        Err(e) => Ok(json!({ "error": say(&core, &e) })),
    }
}

/// Asks for a playlist file (CSV, Spotify JSON export, M3U, XSPF) and imports
/// it into `playlist_id` (if given) or as a new playlist.
pub async fn import_playlist_file(core: C<'_>, playlist_id: Option<String>) -> Res {
    let tr = need_transfer(core)?;
    let core = core.clone();
    let title = crate::i18n::tr(&core, "import.file.pick");
    let Some(path) = core
        .host
        .pick_file(Some(&title), "Playlist", &["csv", "json", "m3u", "m3u8", "xspf", "txt"])
        .await?
    else {
        return Ok(());
    };
    crate::rt::spawn(async move {
        match import_and_save(&core, &tr, ImportInput::File(path), None, playlist_id.as_deref()).await {
            Ok(s) => {
                saved_toast(&core, &s);
                if playlist_id.is_none() {
                    core.emit("open_playlist", s.id.clone());
                }
            }
            Err(e) => failed_toast(&core, &e),
        }
    });
    Ok(())
}

/// Text pasted into the search bar or shared to Glyph. Says what it was:
/// `{ kind: "migrate", text }` (many links: the UI opens Transfer music with
/// it), `"song"`, `"playlist"`, `"share"` or `"none"` (just a search).
/// `ask`: the text was shared to Glyph (share sheet, "Send to", a copied
/// link): one song link is not liked at once but answered as
/// `{ kind: "pick", tracks }`, and the UI asks where it goes.
pub async fn import_text(core: C<'_>, text: String, ask: Option<bool>) -> Res<Value> {
    let tr = need_transfer(core)?;
    let core = core.clone();
    let q = text.trim().to_string();
    // A jam invite (link or code): join it.
    let jam_like = q.contains("/jam/") || q.starts_with(glyph_protocol::jam::CODE_PREFIX);
    if jam_like && glyph_protocol::jam::normalize_code(&q).is_some() {
        crate::jam::open_link(&core, &q);
        return Ok(json!({ "kind": "jam" }));
    }
    if glyph_core::share::is_code(&q) || glyph_core::share::link_id(&q).is_some() {
        return Ok(import_share(&core, &q).await);
    }
    // Many links at once (songs copied in Spotify): Transfer music.
    if q.split_whitespace().count() > 1 {
        let (songs, lists) = Backend::pasted_links(&q);
        if songs + lists > 1 {
            return Ok(json!({ "kind": "migrate", "text": q }));
        }
    }
    // One song link: that song into Liked Songs (or asked where it goes).
    if Backend::is_song_link(&q) {
        if ask.unwrap_or(false) {
            return Ok(match read_song_link(&core, &q).await {
                Some(song) => json!({ "kind": "pick", "tracks": [song] }),
                None => json!({ "kind": "none" }),
            });
        }
        import_song_link(&core, &q).await;
        return Ok(json!({ "kind": "song" }));
    }
    use glyph_core::import::RemoteSource as R;
    if glyph_core::import::parse_url(&q).is_some_and(|s| !matches!(s, R::SpotifyTrack(_) | R::DeezerTrack(_))) {
        return Ok(match import_and_save(&core, &tr, ImportInput::Url(q), None, None).await {
            Ok(s) => {
                saved_toast(&core, &s);
                json!({ "kind": "playlist", "id": s.id })
            }
            Err(e) => {
                failed_toast(&core, &e);
                json!({ "kind": "none" })
            }
        });
    }
    if q.starts_with("http://") || q.starts_with("https://") {
        core.toast_key("import.unknown_link", "info");
    }
    Ok(json!({ "kind": "none" }))
}

/// The song behind one song link, for the "add shared song" sheet.
async fn read_song_link(core: &Arc<Core>, link: &str) -> Option<glyph_protocol::Track> {
    let Ok(b) = core.backend_or_err() else {
        core.toast_key("status.backend_offline", "info");
        return None;
    };
    core.toast_key("import.song.reading", "info");
    match guard_async(b.songs_from_links(link)).await.map(|s| s.into_iter().next()) {
        Ok(Some(song)) => Some(song),
        Ok(None) => {
            core.toast_key("import.song.unreadable", "info");
            None
        }
        Err(e) => {
            toast(core, "import.failed", json!({ "e": e }), None, "info");
            None
        }
    }
}

/// One song link: the song goes into Liked Songs.
async fn import_song_link(core: &Arc<Core>, link: &str) {
    let Ok(b) = core.backend_or_err() else {
        core.toast_key("status.backend_offline", "info");
        return;
    };
    core.toast_key("import.song.reading", "info");
    match guard_async(b.songs_from_links(link)).await.map(|s| s.into_iter().next()) {
        Ok(Some(song)) => {
            let sub = format!("{} · {}", song.title, song.artist);
            let liked = core.st().library.is_liked(&song);
            if liked {
                toast(core, "import.song.known", Value::Null, Some(sub), "info");
            } else {
                core.st().library.toggle_like(&song);
                core.library_changed();
                toast(core, "import.song.added", Value::Null, Some(sub), "ok");
                // Liked Songs that download new songs by themselves start this one now.
                if core.st().library.liked_auto_download {
                    crate::storage::download_tracks(core, vec![song], true);
                }
            }
        }
        Ok(None) => core.toast_key("import.song.unreadable", "info"),
        Err(e) => toast(core, "import.failed", json!({ "e": e }), None, "info"),
    }
}

/// A pasted share code or a `glyph.my/p/..` link. A code is saved at once.
/// A link is shown first as a preview (the UI's list page with "Add to
/// library"; `share_save` saves it), an artist link opens the artist.
async fn import_share(core: &Arc<Core>, text: &str) -> Value {
    let Some(id) = glyph_core::share::link_id(text) else {
        return match save_share(core, text, None, None) {
            Some(id) => json!({ "kind": "playlist", "id": id }),
            None => json!({ "kind": "none" }),
        };
    };
    let link = format!("https://glyph.my/p/{id}");
    let Ok(b) = core.backend_or_err() else {
        core.toast_key("status.library_offline", "info");
        return json!({ "kind": "none" });
    };
    let failed = |core: &Core| {
        core.toast_key("toast.share_link_failed", "info");
        json!({ "kind": "none" })
    };
    match guard_async(b.resolve_share_link(&link)).await {
        Ok(p) if glyph_core::share::is_code(&p.code) => {
            let author = p.author.filter(|_| p.kind != "album");
            match glyph_core::share::decode(&p.code) {
                Ok((name, _)) if p.kind == "artist" => json!({ "kind": "artist", "name": name }),
                Ok((name, tracks)) => {
                    // Saved before: just open it.
                    let known = core.st().library.playlists.iter().find(|x| x.origin.as_deref() == Some(link.as_str())).map(|x| x.id.clone());
                    match known {
                        Some(id) => json!({ "kind": "playlist", "id": id }),
                        None => json!({ "kind": "share_preview", "name": name, "tracks": tracks, "code": p.code, "link": link, "author": author }),
                    }
                }
                Err(_) => failed(core),
            }
        }
        Ok(_) => failed(core),
        Err(e) => {
            tracing::info!(error = %e, "playlist link failed");
            failed(core)
        }
    }
}

/// Adds the playlist of a share code; `origin` / `shared_by` come from a
/// short link (its URL and the author's handle). Returns the playlist's id.
fn save_share(core: &Core, code: &str, origin: Option<String>, shared_by: Option<String>) -> Option<String> {
    match glyph_core::share::decode(code) {
        Ok((name, tracks)) => {
            let known = origin.as_ref().and_then(|link| core.st().library.playlists.iter().find(|p| p.origin.as_ref() == Some(link)).map(|p| p.id.clone()));
            if known.is_some() {
                return known;
            }
            let n = tracks.len();
            let id = {
                let mut st = core.st();
                let p = st.library.create_playlist(&name);
                p.tracks = tracks;
                p.origin = origin;
                p.shared_by = shared_by;
                p.id.clone()
            };
            core.library_changed();
            toast(core, "toast.share_imported", json!({ "n": n }), Some(name), "ok");
            Some(id)
        }
        Err(e) => {
            core.emit("toast", json!({ "text": format!("{e:#}"), "kind": "info" }));
            None
        }
    }
}

// ---- the phone's share sheet ----

/// The backend, waiting for it while the app starts (the share sheet can be
/// what started it).
async fn backend_soon(core: &Arc<Core>) -> Option<Arc<Backend>> {
    for _ in 0..120 {
        if let Some(b) = core.backend() {
            return Some(b);
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    None
}

fn sheet_error(core: &Core, e: &str) -> Value {
    json!({ "kind": "none", "error": say(core, e) })
}

/// What text shared to Glyph holds, read for the phone's share sheet
/// without saving anything: the sheet saves what is ticked.
///
/// `kind` is `songs` (`tracks`), `list` (`name`, `tracks`, `origin`; a
/// glyph.my playlist also `code`, `link`, `author`), `app` (the app
/// itself handles it: a jam, several links, a search) or `none` (`error`).
/// While a list is read, `share_progress` events count its songs.
pub async fn share_read(core: C<'_>, text: String) -> Res<Value> {
    let core = core.clone();
    let Some(q) = crate::links::shared_query(&text) else {
        return Ok(sheet_error(&core, "import.unknown_link"));
    };
    let jam_like = q.contains("/jam/") || q.starts_with(glyph_protocol::jam::CODE_PREFIX);
    if jam_like && glyph_protocol::jam::normalize_code(&q).is_some() {
        return Ok(json!({ "kind": "app" }));
    }
    if glyph_core::share::is_code(&q) || glyph_core::share::link_id(&q).is_some() {
        return Ok(read_share(&core, &q).await);
    }
    // Several links at once go to "move from another app" in the app.
    if q.split_whitespace().count() > 1 {
        let (songs, lists) = Backend::pasted_links(&q);
        if songs + lists > 1 {
            return Ok(json!({ "kind": "app" }));
        }
    }
    use glyph_core::import::RemoteSource as R;
    let song = Backend::is_song_link(&q);
    let list = glyph_core::import::parse_url(&q).is_some_and(|s| !matches!(s, R::SpotifyTrack(_) | R::DeezerTrack(_)));
    if !song && !list {
        // Plain text or an unknown link: searched for in the app.
        return Ok(json!({ "kind": "app" }));
    }
    let Some(b) = backend_soon(&core).await else {
        return Ok(sheet_error(&core, "status.backend_offline"));
    };
    if song {
        return Ok(match guard_async(b.songs_from_links(&q)).await {
            Ok(tracks) if !tracks.is_empty() => json!({ "kind": "songs", "tracks": tracks }),
            Ok(_) => sheet_error(&core, "import.song.unreadable"),
            Err(e) => sheet_error(&core, &e),
        });
    }
    let (tx, mut rx) = flume_like::channel::<ImportProgress>();
    let c2 = core.clone();
    let drain = crate::rt::spawn(async move {
        while let Some(p) = rx.recv().await {
            c2.emit("share_progress", json!({ "done": p.done, "total": p.total }));
        }
    });
    let r = guard_async(b.import(ImportInput::Url(q.clone()), tx)).await;
    drain.abort();
    Ok(match r {
        Ok(res) if !res.ordered.is_empty() => {
            json!({ "kind": "list", "name": res.name, "tracks": res.ordered, "origin": res.origin })
        }
        Ok(_) => sheet_error(&core, "import.empty"),
        Err(e) => sheet_error(&core, &e),
    })
}

/// A playlist code or glyph.my link, read for the share sheet.
async fn read_share(core: &Arc<Core>, text: &str) -> Value {
    let Some(id) = glyph_core::share::link_id(text) else {
        return match glyph_core::share::decode(text) {
            Ok((name, tracks)) => json!({ "kind": "list", "name": name, "tracks": tracks, "code": text }),
            Err(_) => sheet_error(core, "toast.share_link_failed"),
        };
    };
    let link = format!("https://glyph.my/p/{id}");
    let Some(b) = backend_soon(core).await else {
        return sheet_error(core, "status.backend_offline");
    };
    match guard_async(b.resolve_share_link(&link)).await {
        Ok(p) if p.kind == "artist" => json!({ "kind": "app" }),
        Ok(p) if glyph_core::share::is_code(&p.code) => match glyph_core::share::decode(&p.code) {
            Ok((name, tracks)) => {
                let author = p.author.filter(|_| p.kind != "album");
                json!({ "kind": "list", "name": name, "tracks": tracks, "code": p.code, "link": link, "author": author })
            }
            Err(_) => json!({ "kind": "none", "error": crate::i18n::tr(core, "toast.share_link_failed") }),
        },
        Ok(_) => json!({ "kind": "none", "error": crate::i18n::tr(core, "toast.share_link_failed") }),
        Err(e) => {
            tracing::info!(error = %e, "playlist link failed");
            json!({ "kind": "none", "error": crate::i18n::tr(core, "toast.share_link_failed") })
        }
    }
}

/// "Save as playlist" in the share sheet: a list read by [`share_read`]
/// (a link imported before only gets its new songs).
pub fn share_save_list(core: C<'_>, name: String, origin: Option<String>, tracks: Vec<glyph_protocol::Track>) -> Value {
    let s = save_tracks(core, &name, origin.as_deref(), tracks, None, String::new());
    json!({ "id": s.id, "name": s.name, "added": s.added })
}

/// Audio files opened with or shared to Glyph, read for the share sheet.
pub async fn share_read_files(core: C<'_>, paths: Vec<String>) -> Value {
    let paths = paths.into_iter().map(PathBuf::from).collect();
    match crate::opened::read_files(core, paths).await {
        Ok(tracks) => json!({ "kind": "songs", "tracks": tracks }),
        Err(key) => json!({ "kind": "none", "error": crate::i18n::tr(core, key) }),
    }
}

/// "Add to library" on a shared playlist preview: saves it, returns its id.
pub fn share_save(core: C<'_>, code: String, link: Option<String>, author: Option<String>) -> Option<String> {
    save_share(&core, &code, link, author)
}

// ---- moving a library ----

fn service_of(key: &str) -> Res<Service> {
    Service::ALL.into_iter().find(|s| s.key() == key).ok_or_else(|| format!("unknown service {key}"))
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Liked => "liked",
        Kind::Playlist => "playlist",
        Kind::Albums => "albums",
        Kind::Artists => "artists",
    }
}

/// Display name of a collection (as the UI's collectionName, in English).
fn collection_name(core: &Core, c: &Collection) -> String {
    match c.kind {
        Kind::Liked => crate::i18n::tr(core, "migrate.liked"),
        Kind::Albums => crate::i18n::tr(core, "migrate.albums"),
        Kind::Artists => crate::i18n::tr(core, "migrate.artists"),
        Kind::Playlist if !c.name.trim().is_empty() => c.name.clone(),
        Kind::Playlist => c.hint.clone(),
    }
}

fn collection_json(c: &Collection) -> Value {
    json!({
        "id": c.id, "kind": kind_name(c.kind), "name": c.name, "hint": c.hint,
        "count": c.count, "followed": c.followed, "default_on": c.default_on,
    })
}

/// Where files handed over by the UI wait (a stopped move reads them again).
fn files_dir() -> PathBuf {
    glyph_core::paths::data_dir().join("migrate-files")
}

#[derive(Deserialize)]
pub struct PickedFile {
    name: String,
    text: String,
}

/// Writes the files the UI read (name + text) to disk; the core reads paths.
fn write_files(files: Vec<PickedFile>) -> Res<Vec<PathBuf>> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let mut out = Vec::new();
    for (i, f) in files.into_iter().enumerate() {
        let name: String = f.name.rsplit(['/', '\\']).next().unwrap_or("list").chars().map(|c| if c.is_control() || "<>:\"|?*".contains(c) { '_' } else { c }).collect();
        let name = if name.trim().is_empty() { "list.csv".to_string() } else { name };
        // One folder per file: the list is named after the file.
        let dir = files_dir().join(stamp.to_string()).join(i.to_string());
        std::fs::create_dir_all(&dir).map_err(err)?;
        let path = dir.join(name);
        std::fs::write(&path, f.text).map_err(err)?;
        out.push(path);
    }
    Ok(out)
}

/// The login of a stopped move.
fn login_of(st: &MigrationState) -> Option<Login> {
    match (&st.pasted, &st.profile) {
        (Some(text), _) => Some(Login::Pasted { text: text.clone(), files: st.files.clone() }),
        (None, Some(p)) => Some(Login::Public(p.clone())),
        (None, None) if !st.files.is_empty() => Some(Login::Pasted { text: String::new(), files: st.files.clone() }),
        _ => None,
    }
}

/// An unfinished move: `{ service, done, all, account }` or nothing.
pub fn migrate_resume_state() -> Option<Value> {
    let st = MigrationState::load()?;
    Some(json!({ "service": st.service.key(), "done": st.done.len(), "all": st.selected.len(), "account": st.account }))
}

/// Forgets a stopped move.
pub fn migrate_discard(core: C<'_>) {
    MigrationState::clear();
    if !core.transfer().is_some_and(|tr| tr.running()) {
        let _ = std::fs::remove_dir_all(files_dir());
    }
}

/// How many (songs, lists) the links in `text` are.
pub fn migrate_pasted(text: String) -> (usize, usize) {
    Backend::pasted_links(&text)
}

/// Reads what was handed over and says what can come over.
pub async fn migrate_overview(
    core: C<'_>,
    service: String,
    profile: Option<String>,
    pasted: Option<String>,
    files: Option<Vec<PickedFile>>,
    resume: Option<bool>,
) -> Res<Value> {
    let tr = need_transfer(core)?;
    let service = service_of(&service)?;
    let b = core.backend_or_err().map_err(|_| crate::i18n::tr(&core, "status.library_offline"))?;
    let stopped = MigrationState::load().filter(|s| s.service == service);
    let login = if resume.unwrap_or(false) {
        stopped.as_ref().and_then(login_of).ok_or_else(|| crate::i18n::tr(&core, "migrate.error.resume"))?
    } else if let Some(p) = profile.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Login::Public(p.to_string())
    } else {
        if !tr.running() && stopped.is_none() {
            let _ = std::fs::remove_dir_all(files_dir());
        }
        Login::Pasted { text: pasted.unwrap_or_default(), files: write_files(files.unwrap_or_default())? }
    };
    let keys = core.st().config.keys.clone();
    let source = b.library_source(service, login.clone(), keys).map_err(|e| format!("{e:#}"))?;
    let overview = guard_async(b.library_overview(source.clone())).await?;
    let out = json!({
        "account": overview.account,
        "collections": overview.collections.iter().map(collection_json).collect::<Vec<_>>(),
        "notes": overview.notes,
        "done_before": stopped.map(|s| s.done).unwrap_or_default(),
    });
    tr.lock().choice = Some(Choice { service, source, overview, login });
    Ok(out)
}

/// Starts the move of the picked collections; it runs in the background.
pub async fn migrate_start(
    core: C<'_>,
    service: String,
    selected: Vec<String>,
    targets: Option<HashMap<String, MigrateTarget>>,
) -> Res {
    let tr = need_transfer(core)?;
    let core = core.clone();
    let service = service_of(&service)?;
    let b = core.backend_or_err().map_err(|_| crate::i18n::tr(&core, "status.library_offline"))?;
    if tr.running() {
        return Err(crate::i18n::tr(&core, "migrate.error.busy"));
    }
    let choice = {
        let mut g = tr.lock();
        match g.choice.take() {
            Some(c) if c.service == service => c,
            other => {
                g.choice = other;
                return Err(crate::i18n::tr(&core, "migrate.error.choose"));
            }
        }
    };
    // Liked songs first, then playlists, albums, artists.
    let mut picked: Vec<Collection> = choice.overview.collections.iter().filter(|c| selected.contains(&c.id)).cloned().collect();
    picked.sort_by_key(|c| match c.kind {
        Kind::Liked => 0,
        Kind::Playlist => 1,
        Kind::Albums => 2,
        Kind::Artists => 3,
    });
    if picked.is_empty() {
        tr.lock().choice = Some(choice);
        return Err(crate::i18n::tr(&core, "migrate.error.pick"));
    }
    let (profile, pasted, files) = match &choice.login {
        Login::Public(p) => (Some(p.clone()), None, Vec::new()),
        Login::Pasted { text, files } => (None, Some(text.clone()).filter(|t| !t.is_empty()), files.clone()),
    };
    let state = MigrationState {
        service,
        profile,
        pasted,
        files,
        account: choice.overview.account.clone(),
        selected: picked.clone(),
        done: Vec::new(),
        started: glyph_core::paths::now_unix(),
    };
    if let Err(e) = state.save() {
        tracing::warn!(error = %e, "migration state");
    }
    let cancel = Arc::new(AtomicBool::new(false));
    tr.lock().run = Some(Run {
        service,
        index: 0,
        lists: picked.len(),
        current: String::new(),
        done: 0,
        total: 0,
        totals: Totals::default(),
        targets: targets.unwrap_or_default(),
        failed: Vec::new(),
        notes: choice.overview.notes.clone(),
        cancel: cancel.clone(),
        state,
        finished: false,
        cancelled: false,
    });

    let (tx, mut rx) = flume_like::channel::<MigrationEvent>();
    {
        let (core, tr) = (core.clone(), tr.clone());
        crate::rt::spawn(async move {
            while let Some(ev) = rx.recv().await {
                on_event(&core, &tr, ev);
            }
        });
    }
    let source = choice.source;
    crate::rt::spawn(async move {
        if let Err(e) = guard_async(b.migrate(source, picked, tx, cancel)).await {
            // The run ended without a "finished" event: say why, keep the
            // saved state so it can continue.
            let mut g = tr.lock();
            if let Some(r) = g.run.as_mut().filter(|r| !r.finished) {
                r.failed.push((String::new(), e));
                r.finished = true;
            }
        }
    });
    Ok(())
}

fn on_event(core: &Arc<Core>, tr: &Transfer, ev: MigrationEvent) {
    let mut g = tr.lock();
    let Some(run) = g.run.as_mut().filter(|r| !r.finished) else { return };
    match ev {
        MigrationEvent::Collection { index, total, collection } => {
            run.index = index;
            run.lists = total;
            run.current = collection_name(core, &collection);
            run.done = 0;
            run.total = 0;
        }
        MigrationEvent::Progress { done, total } => {
            run.done = done;
            run.total = total;
        }
        MigrationEvent::Failed { collection, error } => {
            run.failed.push((collection_name(core, &collection), error));
            run.state.mark_done(&collection.id);
            let _ = run.state.save();
        }
        MigrationEvent::Done { collection, result } => {
            let target = run.targets.get(&collection.id).cloned();
            let mut stats_col = collection.clone();
            if let Some(ref tgt) = target {
                stats_col.kind = match tgt {
                    MigrateTarget::Liked => Kind::Liked,
                    MigrateTarget::Playlist { .. } | MigrateTarget::NewPlaylist { .. } => Kind::Playlist,
                };
            }
            run.totals.add(&stats_col, &result);
            run.state.mark_done(&collection.id);
            let _ = run.state.save();
            apply_migrated(core, &collection, result, target.as_ref());
        }
        MigrationEvent::Finished { cancelled } => {
            run.finished = true;
            run.cancelled = cancelled;
            if !cancelled {
                MigrationState::clear();
                let _ = std::fs::remove_dir_all(files_dir());
            }
        }
    }
}

fn fallback_list_name(core: &Core, c: &Collection, parsed_name: &str) -> String {
    if !c.name.trim().is_empty() {
        c.name.clone()
    } else if !parsed_name.trim().is_empty() {
        parsed_name.to_string()
    } else {
        crate::i18n::tr(core, "import.unnamed_playlist")
    }
}

/// Saves one moved collection into the library.
fn apply_migrated(core: &Core, c: &Collection, result: Migrated, target: Option<&MigrateTarget>) {
    {
        let mut st = core.st();
        match result {
            Migrated::Tracks(res) => match target {
                Some(MigrateTarget::Liked) => {
                    st.library.like_all(res.ordered);
                }
                Some(MigrateTarget::Playlist { id }) => {
                    if st.library.add_imported_to_playlist(id, res.ordered.clone()).is_none() {
                        let name = fallback_list_name(core, c, &res.name);
                        st.library.save_import(&name, c.origin.as_deref(), res.ordered);
                    }
                }
                Some(MigrateTarget::NewPlaylist { name }) => {
                    let custom = name.as_deref().map(str::trim).filter(|n| !n.is_empty());
                    let list_name = custom.map(str::to_string).unwrap_or_else(|| fallback_list_name(core, c, &res.name));
                    st.library.save_import(&list_name, c.origin.as_deref(), res.ordered);
                }
                None => match c.kind {
                    Kind::Liked => {
                        st.library.like_all(res.ordered);
                    }
                    _ => {
                        let name = fallback_list_name(core, c, &res.name);
                        st.library.save_import(&name, c.origin.as_deref(), res.ordered);
                    }
                },
            },
            Migrated::Albums(albums) => {
                st.library.save_albums(albums.into_iter().map(|a| (a.title, a.artist, a.tracks)));
            }
            Migrated::Artists(names) => {
                st.library.follow_all(names);
            }
        }
    }
    core.library_changed();
}

/// Where the move is (the UI asks twice a second while one runs).
pub fn migrate_progress(core: C<'_>) -> Value {
    let Some(tr) = core.transfer() else {
        return json!({
            "state": "idle", "index": 0, "lists": 0, "current": "", "done": 0, "total": 0,
            "totals": totals_json(&Totals::default(), 0), "failed": [], "notes": [],
            "cancelled": false, "stopping": false, "bar": "",
        });
    };
    let g = tr.lock();
    let Some(r) = g.run.as_ref() else {
        return json!({
            "state": "idle", "index": 0, "lists": 0, "current": "", "done": 0, "total": 0,
            "totals": totals_json(&Totals::default(), 0), "failed": [], "notes": [],
            "cancelled": false, "stopping": false, "bar": "",
        });
    };
    let filled = if r.total == 0 { 0 } else { (r.done * 10 / r.total).min(10) };
    let bar = format!("[{}{}] {}/{}", "#".repeat(filled), "-".repeat(10 - filled), r.done, r.total);
    json!({
        "state": if r.finished { "finished" } else { "running" },
        "index": r.index, "lists": r.lists, "current": r.current, "done": r.done, "total": r.total,
        "totals": totals_json(&r.totals, r.failed.len()),
        "failed": r.failed, "notes": r.notes,
        "cancelled": r.cancelled, "stopping": !r.finished && r.cancel.load(Ordering::Relaxed),
        "bar": bar,
    })
}

fn totals_json(t: &Totals, failed: usize) -> Value {
    json!({
        "songs": t.songs, "matched": t.matched, "unsure": t.unsure, "liked": t.liked,
        "playlists": t.playlists, "albums": t.albums, "artists": t.artists, "failed": failed,
    })
}

/// Stops after the song being matched; what is saved stays.
pub fn migrate_cancel(core: C<'_>) {
    if let Some(tr) = core.transfer() {
        if let Some(r) = tr.lock().run.as_ref().filter(|r| !r.finished) {
            r.cancel.store(true, Ordering::Relaxed);
        }
    }
}
