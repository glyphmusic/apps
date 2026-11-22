//! The commands the Svelte UI calls (`backend().call(name, args)`).
//! Arguments arrive in snake_case, as the UI writes them. The full list the
//! UI uses is in `glyph-ui/src/lib/api/demo*.ts`; `docs/TAURI.md` says
//! which ones are here yet.

use crate::controller::{Core, PlayFrom, SearchCache};
use crate::dto;
use futures::StreamExt;
use glyph_core::{Config, ListKey, SearchItem};
use glyph_protocol::{SearchEvent, Track};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

type Res<T = ()> = Result<T, String>;
type C<'a> = &'a Arc<Core>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ---- start ----

pub fn snapshot(core: C) -> Value {
    core.snapshot()
}

// ---- playback ----

pub fn play(core: C, tracks: Vec<Track>, index: usize, context: Option<String>, from: Option<PlayFrom>) {
    core.play_list_from(tracks, index, context, from.unwrap_or_default());
}

pub fn play_shuffled(core: C, tracks: Vec<Track>, context: Option<String>, from: Option<PlayFrom>) {
    core.play_shuffled(tracks, context, from.unwrap_or_default());
}

pub fn previous_song(core: C) {
    core.previous_song();
}

pub fn play_recent(core: C) {
    core.play_recent();
}

pub fn toggle_play(core: C) {
    core.toggle_play();
}

pub fn next(core: C) {
    core.next(false);
}

pub fn previous(core: C) {
    core.previous();
}

pub fn seek(core: C, position: f64) {
    core.seek(position);
}

pub fn set_volume(core: C, volume: f32) {
    core.set_volume(volume);
}

pub fn toggle_mute(core: C) {
    core.toggle_mute();
}

pub fn set_shuffle(core: C, on: bool) {
    core.set_shuffle(on);
}

pub fn set_repeat(core: C, mode: String) {
    core.set_repeat(&mode);
}

pub fn set_loop(core: C, a: Option<f64>, b: Option<f64>) {
    core.set_loop(a, b);
}

// ---- queue ----

pub fn queue_add(core: C, tracks: Vec<Track>, next: bool) {
    core.queue_add(tracks, next);
}

pub fn queue_remove(core: C, index: usize) {
    core.queue_remove(index);
}

pub fn queue_move(core: C, from: usize, to: usize) {
    core.queue_move(from, to);
}

pub fn queue_jump(core: C, index: usize) {
    core.queue_jump(index);
}

pub fn queue_clear(core: C) {
    core.queue_clear();
}

// ---- library (src/app/library.rs, lists.rs) ----

/// Runs `f` on the library, then saves and tells the UI.
fn lib<R>(core: &Core, f: impl FnOnce(&mut glyph_core::Library) -> R) -> R {
    let r = f(&mut core.st().library);
    core.library_changed();
    r
}

pub fn like(core: C, track: Track, on: bool) {
    lib(&core, |l| {
        if l.is_liked(&track) != on {
            // Liking counts a little for the Liked Songs place in the sidebar.
            if on {
                l.record_list_use(&ListKey::Liked, 0.3, glyph_core::paths::now_unix());
            }
            l.toggle_like(&track);
        }
    });
}

/// Likes or unlikes many songs in one go (clearing Liked Songs, removing a
/// selection, or many hearts tapped quickly): one save, one library update
/// for the UI and one sync, however many songs. One update per song made the
/// UI redraw the whole library again and again (a black screen on phones).
pub fn like_many(core: C, tracks: Vec<Track>, on: bool) {
    if tracks.is_empty() {
        return;
    }
    lib(&core, |l| {
        if on {
            l.record_list_use(&ListKey::Liked, 0.3, glyph_core::paths::now_unix());
        }
        for t in &tracks {
            if l.is_liked(t) != on {
                l.toggle_like(t);
            }
        }
    });
}

/// Clears all Liked Songs at once so sync sends a single `liked_clear` signal
/// instead of one deletion per song.
pub fn clear_liked(core: C) {
    lib(&core, |l| {
        if !l.liked.is_empty() {
            l.clear_liked(glyph_core::paths::now_ms());
        }
    });
}

pub fn playlist_create(core: C, name: String, tracks: Vec<Track>) -> String {
    let limit = glyph_core::library::playlist_track_limit();
    let over = tracks.len() > limit;
    let id = lib(&core, |l| {
        let id = l.create_playlist(&name).id.clone();
        for t in tracks.into_iter().take(limit) {
            l.add_to_playlist(&id, t);
        }
        id
    });
    if over {
        core.toast_vars("toast.playlist_limit", json!({ "n": limit }), "info");
    }
    id
}

pub fn playlist_rename(core: C, id: String, name: String) {
    lib(&core, |l| {
        l.record_list_use(&ListKey::Playlist { id: id.clone() }, 0.3, glyph_core::paths::now_unix());
        l.rename_playlist(&id, &name)
    });
}

pub fn playlist_delete(core: C, id: String) {
    lib(&core, |l| l.delete_playlist(&id));
}

pub fn playlist_restore(core: C, id: String) -> bool {
    lib(&core, |l| l.restore_playlist(&id))
}

pub fn playlist_purge(core: C, id: String) -> bool {
    lib(&core, |l| l.purge_playlist(&id))
}

pub fn playlist_add(core: C, id: String, tracks: Vec<Track>) -> usize {
    // A server may limit the songs of a playlist.
    let limit = glyph_core::library::playlist_track_limit();
    let full = {
        let st = core.st();
        st.library.playlist(&id).is_some_and(|p| {
            let mut seen = HashSet::new();
            let new = tracks
                .iter()
                .filter(|t| !p.tracks.iter().any(|x| x.id == t.id && x.source == t.source))
                .filter(|t| seen.insert(format!("{:?}:{}", t.source, t.id)))
                .count();
            p.tracks.len().saturating_add(new) > limit
        })
    };
    if full {
        core.toast_vars("toast.playlist_limit", json!({ "n": limit }), "info");
        return 0;
    }
    lib(&core, |l| {
        l.record_list_use(&ListKey::Playlist { id: id.clone() }, 0.5, glyph_core::paths::now_unix());
        tracks.into_iter().filter(|t| l.add_to_playlist(&id, t.clone())).count()
    })
}

pub fn playlist_remove(core: C, id: String, index: usize) {
    lib(&core, |l| l.remove_from_playlist(&id, index));
}

/// Removes several songs of a playlist (a selection) with one save and one
/// library update. `indices` are positions in the playlist as it is now.
pub fn playlist_remove_many(core: C, id: String, mut indices: Vec<usize>) {
    if indices.is_empty() {
        return;
    }
    // From the back, so the positions still to remove stay where they are.
    indices.sort_unstable_by(|a, b| b.cmp(a));
    indices.dedup();
    lib(&core, |l| {
        for i in indices {
            l.remove_from_playlist(&id, i);
        }
    });
}

pub fn playlist_move(core: C, id: String, from: usize, to: usize) {
    if from == to {
        return;
    }
    lib(&core, |l| {
        l.record_list_use(&ListKey::Playlist { id: id.clone() }, 0.3, glyph_core::paths::now_unix());
        l.move_in_playlist(&id, from, to)
    });
}

pub fn list_insert(core: C, list: ListKey, index: usize, track: Track) {
    lib(&core, |l| match list {
        ListKey::Liked => l.insert_liked(index, track),
        ListKey::Playlist { id } => {
            l.insert_into_playlist(&id, index, track);
        }
        ListKey::Recent => l.insert_recent_song(index, track),
        _ => {}
    });
}

pub fn pin(core: C, list: ListKey, on: bool) {
    lib(&core, |l| {
        l.pinned.retain(|k| !k.same(&list));
        if on {
            l.pinned.insert(0, list);
        }
    });
}

pub fn save_album(core: C, list: ListKey, on: bool) {
    lib(&core, |l| {
        if l.is_saved(&list) != on {
            l.toggle_saved(list);
        }
    });
}

pub fn set_follow(core: &Arc<Core>, artist: String, on: bool) {
    let changed = lib(core, |l| {
        let c = l.is_following(&artist) != on;
        if c {
            l.toggle_follow(&artist, None);
        }
        c
    });
    if changed {
        follow_changed(core, artist, on);
    }
}

pub fn follow(core: C, artist: String, on: bool) {
    set_follow(core, artist, on);
}

pub fn hide_artist(core: C, artist: String, on: bool) {
    lib(&core, |l| {
        if l.is_hidden(&artist) != on {
            l.toggle_hidden(&artist);
        }
    });
    // Their songs leave the results on screen at once.
    if on {
        let mut st = core.st();
        let keep: Vec<bool> = st.search.tracks.iter().map(|t| !glyph_core::dedupe::by_artist(&t.artist, &artist)).collect();
        let mut k = keep.iter();
        st.search.tracks.retain(|_| *k.next().unwrap_or(&true));
        let mut k = keep.iter();
        st.search.scores.retain(|_| *k.next().unwrap_or(&true));
    }
}

pub fn hide_sidebar(core: C, id: String) {
    lib(&core, |l| {
        l.toggle_sidebar_hidden(&id);
    });
}

pub fn recent_remove(core: C, index: usize) {
    lib(&core, |l| {
        if index < l.recent.len() {
            l.recent.remove(index);
        }
    });
}

pub fn recent_remove_track(core: C, track: Track) {
    lib(&core, |l| l.remove_recent_song(&track));
}

pub fn list_opened(core: C, list: ListKey) {
    // The first songs get their sources looked up now, so Play starts at once.
    if let Some(b) = core.backend() {
        let first: Vec<Track> = {
            let st = core.st();
            match &list {
                ListKey::Liked => st.library.liked.iter().take(2).cloned().collect(),
                ListKey::Recent => st.library.recent_tracks().take(2).cloned().collect(),
                ListKey::Playlist { id } => st.library.playlist(id).map(|p| p.tracks.iter().take(2).cloned().collect()).unwrap_or_default(),
                ListKey::Album { tracks, .. } => tracks.iter().take(2).cloned().collect(),
                _ => Vec::new(),
            }
        };
        if !first.is_empty() {
            b.warm(first);
        }
    }
    lib(&core, |l| {
        l.push_recent_list(list.clone());
        // Opening a list counts a little for its place in the sidebar.
        l.record_list_use(&list, 0.25, glyph_core::paths::now_unix());
    });
}

pub fn search_history_add(core: C, item: SearchItem) {
    lib(&core, |l| match item {
        SearchItem::Song { track } => l.push_search(track),
        SearchItem::Artist { name, track } => l.push_search_artist(&name, track),
        SearchItem::Query { text } => l.push_search_query(&text),
    });
}

pub fn search_history_remove(core: C, index: usize) {
    lib(&core, |l| l.remove_search(index));
}

pub fn lyrics_offset(core: C, track: Track, offset_ms: i64) {
    lib(&core, |l| l.set_lyrics_offset(&track, offset_ms));
}

// ---- search, pages (src/app/search.rs, artist.rs, lists.rs) ----

const SEARCH_SETTLE: Duration = Duration::from_millis(800);
const SEARCH_PINNED: usize = 3;

/// Same song from the same source (what the results are deduplicated by).
fn src_key(t: &Track) -> String {
    format!("{:?}:{}", t.source, t.id)
}

fn hidden_artist(lib: &glyph_core::Library, credit: &str) -> bool {
    !lib.hidden_artists.is_empty() && lib.hidden_artists.iter().any(|h| glyph_core::dedupe::by_artist(credit, h))
}

impl SearchCache {
    fn clear_results(&mut self) {
        self.tracks.clear();
        self.scores.clear();
        self.shown_at = None;
    }

    /// Rows that keep their place.
    fn pinned(&self) -> usize {
        match self.shown_at {
            Some(at) if at.elapsed() >= SEARCH_SETTLE => SEARCH_PINNED.min(self.tracks.len()),
            _ => 0,
        }
    }

    /// Inserts `t` (score `sc`) below everything that scores at least as well.
    fn insert_ranked(&mut self, t: Track, sc: f32) {
        let at = self.scores.iter().position(|&x| x < sc).unwrap_or(self.scores.len()).max(self.pinned());
        self.tracks.insert(at, t);
        self.scores.insert(at, sc);
        self.shown_at.get_or_insert_with(Instant::now);
    }

    /// `t` came again (another provider, better data): it replaces its row
    /// and moves to where its new score puts it (a pinned row stays).
    fn replace_ranked(&mut self, t: Track, sc: f32) {
        let Some(i) = self.tracks.iter().position(|x| x.source == t.source && x.id == t.id) else { return };
        if i < self.pinned() {
            self.tracks[i] = t;
            self.scores[i] = sc;
            return;
        }
        self.tracks.remove(i);
        self.scores.remove(i);
        self.insert_ranked(t, sc);
    }

    /// A later page: below everything already shown.
    fn push_unranked(&mut self, t: Track) {
        self.tracks.push(t);
        self.scores.push(f32::NEG_INFINITY);
    }
}

/// The text a search runs with (operators like `artist:` removed).
fn plain_query(q: &str) -> String {
    glyph_core::taste::parse_query(&glyph_core::search_ops::SearchOps::parse(q.trim()).query()).0
}

/// Artists in the results, best matches for the query first.
fn search_artists(tracks: &[Track], query: &str, lib: &glyph_core::Library, max: usize) -> Vec<Value> {
    let q = plain_query(query).to_lowercase();
    let mut found: Vec<(String, Track, usize)> = Vec::new();
    for tr in tracks {
        let name = glyph_core::dedupe::primary_artist(&tr.artist);
        if name.is_empty() {
            continue;
        }
        match found.iter_mut().find(|(n, _, _)| n.eq_ignore_ascii_case(&name)) {
            Some(f) => f.2 += 1,
            None => found.push((name, tr.clone(), 1)),
        }
    }
    let score = |n: &str, count: usize| -> i32 {
        let l = n.to_lowercase();
        let m = if l == q {
            1000
        } else if l.starts_with(&q) {
            500
        } else if l.contains(&q) || (!l.is_empty() && q.contains(&l)) {
            200
        } else {
            0
        };
        m + count as i32
    };
    // Only artists that match the query, or that clearly dominate the results.
    found.retain(|(n, _, c)| (score(n, 0) > 0 || *c >= 3) && !lib.is_hidden(n));
    found.sort_by_key(|(n, _, c)| -score(n, *c));
    found.into_iter().take(max).map(|(n, t, _)| json!({ "name": n, "track": t })).collect()
}

/// Query completions taken from the results (titles and artists that
/// contain what was typed).
fn search_completions(tracks: &[Track], query: &str, max: usize) -> Vec<String> {
    let q = plain_query(query).to_lowercase();
    if q.len() < 2 {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    for tr in tracks {
        for cand in [tr.title.as_str(), glyph_core::dedupe::primary_artist(&tr.artist).as_str()] {
            // Drop bracketed extras ("(Radio Edit)", "[Official]") for a clean suggestion.
            let clean = cand.split(['(', '[']).next().unwrap_or(cand).trim().to_string();
            let l = clean.to_lowercase();
            if l.len() <= q.len() || !l.contains(&q) || clean.chars().count() > 40 {
                continue;
            }
            if !out.iter().any(|o| o.to_lowercase() == l) {
                out.push(clean);
            }
        }
        if out.len() >= max {
            break;
        }
    }
    out
}

/// The search as the UI shows it.
fn results(core: &Core, loading: bool) -> Value {
    let st = core.st();
    let c = &st.search;
    // Albums: the songs' albums, grouped.
    let mut albums: Vec<Value> = Vec::new();
    let mut seen_albums: Vec<(String, String)> = Vec::new();
    for t in &c.tracks {
        let Some(al) = &t.album else { continue };
        let key = (al.to_lowercase(), t.artist.to_lowercase());
        if seen_albums.contains(&key) {
            continue;
        }
        seen_albums.push(key);
        albums.push(json!({
            "id": format!("album:{al}|{}", t.artist), "title": al, "artist": t.artist, "kind": "album",
            "year": Value::Null, "date": Value::Null, "cover_url": t.cover_url,
        }));
    }
    json!({
        "query": c.query, "tracks": c.tracks,
        "artists": search_artists(&c.tracks, &c.query, &st.library, 6),
        "albums": albums, "suggestions": search_completions(&c.tracks, &c.query, 6),
        "lyric_hits": c.lyric_hits.iter().map(|(t, l)| json!({ "track": t, "line": l })).collect::<Vec<_>>(),
        "loading": loading, "more": !c.exhausted && c.page < 20 && !c.tracks.is_empty() && !loading,
        "error": Value::Null, "errors": c.errors,
    })
}

fn emit_results(core: &Core, loading: bool) {
    let v = results(core, loading);
    core.emit("search", v);
}

/// Offline: only saved songs from the results, library and downloads.
fn search_offline(core: &Core, query: &str) {
    let downloads = core.backend().map(|b| b.downloaded_snapshot()).unwrap_or_default();
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let candidates: Vec<Track> = {
        let st = core.st();
        st.search
            .tracks
            .iter()
            .chain(st.library.liked.iter())
            .chain(st.library.recent_tracks())
            .chain(st.library.playlists.iter().flat_map(|p| p.tracks.iter()))
            .chain(downloads.iter())
            .cloned()
            .collect()
    };
    let mut seen = HashSet::new();
    let found: Vec<Track> = candidates
        .into_iter()
        .filter(|t| core.playable(t) && crate::storage::available_offline(core, t))
        .filter(|t| {
            let text = format!("{} {} {}", t.title, t.artist, t.album.as_deref().unwrap_or("")).to_lowercase();
            terms.iter().all(|w| text.contains(w)) && seen.insert(glyph_core::dedupe::track_key(t))
        })
        .collect();
    let mut st = core.st();
    st.search.clear_results();
    for t in found {
        st.search.push_unranked(t);
    }
    st.search.errors.clear();
    st.search.lyric_hits.clear();
    st.search.exhausted = true;
}

/// A pasted code or link instead of a search (settings code, equalizer
/// code, jam invite, playlist code / link, links of other services):
/// `Some(kind)` where "import" means the UI calls `import_text`.
pub fn paste_kind(q: &str) -> Option<&'static str> {
    let q = q.trim();
    if q.is_empty() {
        return None;
    }
    if glyph_core::settings_share::is_code(q) {
        return Some("settings");
    }
    if glyph_core::eq::is_code(q) {
        return Some("eq");
    }
    if glyph_protocol::jam::is_jam_text(q) || glyph_core::share::is_code(q) || glyph_core::share::link_id(q).is_some() {
        return Some("import");
    }
    if q.split_whitespace().count() > 1 {
        let (songs, lists) = glyph_core::Backend::pasted_links(q);
        if songs + lists > 1 {
            return Some("import");
        }
    }
    if glyph_core::Backend::is_song_link(q) {
        return Some("import");
    }
    use glyph_core::import::RemoteSource as R;
    if glyph_core::import::parse_url(q).is_some_and(|s| !matches!(s, R::SpotifyTrack(_) | R::DeezerTrack(_))) {
        return Some("import");
    }
    None
}

pub fn search_paste_kind(query: String) -> Option<&'static str> {
    paste_kind(&query)
}

/// Runs one search (page 0 starts a new one, later pages add to it); partial
/// results go to the UI as `search` events. Hidden artists, hidden explicit
/// songs and `artist:` / `"phrase"` filters apply; hits are ranked by how
/// well they match the query and the user's taste.
async fn run_search(core: &Arc<Core>, query: &str, page: usize) -> Res<Value> {
    if page > 0 {
        return search_page_more(core, query, page).await;
    }
    let seq = {
        let mut st = core.st();
        let seq = st.search.seq + 1;
        st.search = Default::default();
        st.search.seq = seq;
        st.search.query = query.to_string();
        st.search.loading = true;
        seq
    };
    if crate::storage::is_offline() {
        search_offline(core, query);
        core.st().search.loading = false;
        let v = results(core, false);
        return Ok(v);
    }
    let b = core.backend_or_err()?;
    let ops = glyph_core::search_ops::SearchOps::parse(query);
    let (plain, phrases) = glyph_core::taste::parse_query(&ops.query());
    let taste = {
        let st = core.st();
        if st.config.taste.enabled {
            glyph_core::taste::Taste::from_library(&st.library)
        } else {
            glyph_core::taste::Taste::default()
        }
    };
    // Lines of lyrics also find songs (a separate section, its own request).
    let lq = glyph_core::search_ops::SearchOps::parse(query).text;
    if glyph_core::lyricsearch::looks_like_lyrics(&lq) {
        let (core, b) = (core.clone(), b.clone());
        crate::rt::spawn(async move {
            let hits = b.search_lyrics(&lq).await.unwrap_or_default();
            {
                let mut st = core.st();
                if st.search.seq != seq {
                    return;
                }
                let lib = &st.library;
                let hide = st.config.audio.hide_explicit;
                let kept: Vec<(Track, String)> = hits
                    .into_iter()
                    .filter(|h| !(h.track.maybe_explicit() && hide) && !hidden_artist(lib, &h.track.artist))
                    .map(|h| (h.track, h.snippet))
                    .collect();
                st.search.lyric_hits = kept;
            }
            let loading = core.st().search.loading;
            emit_results(&core, loading);
        });
    }
    let mut stream = b.search(&plain);
    let deadline = Instant::now() + Duration::from_secs(12);
    const EMIT_COALESCE: Duration = Duration::from_millis(120);
    let mut last_emit = Instant::now() - EMIT_COALESCE;
    let mut dirty = false;
    let mut warmed_early = false;
    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let flush_at = last_emit + EMIT_COALESCE;
        let ev = if dirty && now < flush_at {
            tokio::select! {
                res = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), stream.next()) => {
                    match res {
                        Ok(Some(ev)) => Some(ev),
                        _ => None,
                    }
                }
                _ = tokio::time::sleep_until(tokio::time::Instant::from_std(flush_at)) => {
                    dirty = false;
                    last_emit = Instant::now();
                    emit_results(core, true);
                    continue;
                }
            }
        } else {
            tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), stream.next())
                .await
                .ok()
                .flatten()
        };
        let Some(ev) = ev else { break };
        let mut done = false;
        let mut changed = false;
        {
            let mut st = core.st();
            if st.search.seq != seq {
                return Ok(Value::Null);
            }
            match ev {
                SearchEvent::Results { tracks, .. } => {
                    let hide = st.config.audio.hide_explicit;
                    for t in tracks {
                        if hidden_artist(&st.library, &t.artist)
                            || (t.maybe_explicit() && hide)
                            || !glyph_core::taste::matches_phrases(&t, &phrases)
                            || !ops.matches(&t)
                        {
                            continue;
                        }
                        // Ranked by text match, odd versions and the user's
                        // taste; each song is scored once.
                        let sc = glyph_core::taste::score(&t, &plain, &taste);
                        if st.search.seen.insert(src_key(&t)) {
                            st.search.insert_ranked(t, sc);
                        } else {
                            st.search.replace_ranked(t, sc);
                        }
                        changed = true;
                    }
                }
                SearchEvent::ProviderError { provider, message } => {
                    st.search.errors.push(json!({ "source": provider, "message": message }));
                    changed = true;
                }
                SearchEvent::Done => done = true,
            }
        }
        if done {
            break;
        }
        if changed {
            if last_emit.elapsed() >= EMIT_COALESCE {
                dirty = false;
                last_emit = Instant::now();
                emit_results(core, true);
            } else {
                dirty = true;
            }
            // The first hit is the likely click: find its source now, not
            // only once every source has answered (that can take seconds).
            if !warmed_early {
                let first = {
                    let st = core.st();
                    (st.search.seq == seq).then(|| st.search.tracks.first().cloned()).flatten()
                };
                if let Some(t) = first {
                    warmed_early = true;
                    b.warm(vec![t]);
                }
            }
        }
    }
    // The top hits are the likely clicks: find their sources now so they start at once.
    let top: Vec<Track> = {
        let mut st = core.st();
        if st.search.seq != seq {
            return Ok(Value::Null);
        }
        st.search.loading = false;
        st.search.tracks.iter().take(2).cloned().collect()
    };
    b.warm(top);
    let v = results(core, false);
    // AI apps (MCP) find songs from the last search.
    crate::mcp_tools::remember_search(&core.st().search.tracks);
    Ok(v)
}

/// Scrolled near the end of the results: the next page is added below
/// (sources that can page; stops when a page brings nothing new).
async fn search_page_more(core: &Arc<Core>, query: &str, _hint: usize) -> Res<Value> {
    let b = core.backend_or_err()?;
    let (seq, page, plain, ops) = {
        let mut st = core.st();
        let c = &mut st.search;
        if c.query != query || c.loading || c.loading_more || c.exhausted || c.tracks.is_empty() || c.page >= 20 {
            drop(st);
            return Ok(results(core, false));
        }
        c.page += 1;
        c.loading_more = true;
        (c.seq, c.page, plain_query(query), glyph_core::search_ops::SearchOps::parse(query))
    };
    let (_, phrases) = glyph_core::taste::parse_query(&ops.query());
    let mut stream = b.search_page(&plain, page);
    let mut added = 0usize;
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok(Some(ev)) = tokio::time::timeout(left, stream.next()).await else { break };
        match ev {
            SearchEvent::Results { tracks, .. } => {
                let mut st = core.st();
                if st.search.seq != seq {
                    return Ok(Value::Null);
                }
                let hide = st.config.audio.hide_explicit;
                for t in tracks {
                    if hidden_artist(&st.library, &t.artist)
                        || (t.maybe_explicit() && hide)
                        || !glyph_core::taste::matches_phrases(&t, &phrases)
                        || !ops.matches(&t)
                    {
                        continue;
                    }
                    // Later pages go below what is already shown.
                    if st.search.seen.insert(src_key(&t)) {
                        st.search.push_unranked(t);
                        added += 1;
                    }
                }
            }
            SearchEvent::ProviderError { .. } => {}
            SearchEvent::Done => break,
        }
    }
    {
        let mut st = core.st();
        if st.search.seq == seq {
            st.search.loading_more = false;
            st.search.exhausted = added == 0;
        }
    }
    crate::mcp_tools::remember_search(&core.st().search.tracks);
    Ok(results(core, false))
}

pub async fn search(core: C<'_>, query: String, page: Option<usize>) -> Res<Value> {
    let core = core.clone();
    // A pasted code or link is imported instead of searched.
    if let Some(kind) = paste_kind(&query) {
        return Ok(json!({ "paste": kind, "text": query.trim(), "query": query }));
    }
    run_search(&core, &query, page.unwrap_or(0)).await
}

pub async fn search_more(core: C<'_>, query: String, page: Option<usize>) -> Res<Value> {
    run_search(core, &query, page.unwrap_or(1).max(1)).await
}

/// Drops retained Rust-side search buffers when leaving the Search view.
pub fn release_search(core: C) {
    core.clear_search_state();
}

// ---- home feeds (src/app/lists.rs load_feeds, load_new_releases) ----

/// Feeds and releases up to this old are shown while fresh ones load.
const STALE_FEEDS_S: u64 = 7 * 24 * 3600;

fn now_s() -> u64 {
    glyph_core::paths::now_unix()
}

fn scope_of(core: &Core) -> std::path::PathBuf {
    core.st().library.storage_path.0.clone().unwrap_or_else(glyph_core::Library::guest_path)
}

fn feeds_cache_path(scope: &std::path::Path) -> std::path::PathBuf {
    scope.with_extension("feeds.json")
}

fn save_feeds_cache(scope: &std::path::Path, trending: &[Track], recs: &[Track]) {
    if let Ok(bytes) = serde_json::to_vec(&(now_s(), trending, recs)) {
        let _ = glyph_core::paths::write_atomic(&feeds_cache_path(scope), &bytes);
    }
}

/// The cached feeds when younger than `max_age_s` and not empty.
fn load_feeds_cache(scope: &std::path::Path, max_age_s: u64) -> Option<(Vec<Track>, Vec<Track>)> {
    let (at, trending, recs): (u64, Vec<Track>, Vec<Track>) = serde_json::from_slice(&std::fs::read(feeds_cache_path(scope)).ok()?).ok()?;
    (now_s().saturating_sub(at) < max_age_s && !(trending.is_empty() && recs.is_empty())).then_some((trending, recs))
}

fn releases_cache_path(scope: &std::path::Path) -> std::path::PathBuf {
    // v2: entries are checked against the album's main artist.
    scope.with_extension("releases-v2.json")
}

type Release = glyph_core::providers::deezer::Release;

fn save_releases_cache(scope: &std::path::Path, artists: &[String], releases: &[Release]) {
    if let Ok(bytes) = serde_json::to_vec(&(now_s(), artists, releases)) {
        let _ = glyph_core::paths::write_atomic(&releases_cache_path(scope), &bytes);
    }
}

fn load_releases_cache(scope: &std::path::Path, artists: &[String], max_age_s: u64) -> Option<Vec<Release>> {
    let (at, names, releases): (u64, Vec<String>, Vec<Release>) = serde_json::from_slice(&std::fs::read(releases_cache_path(scope)).ok()?).ok()?;
    (now_s().saturating_sub(at) < max_age_s && names == artists).then_some(releases)
}

fn feeds_json(core: &Core) -> Value {
    let st = core.st();
    json!({
        "recs": st.feeds.recs, "trending": st.feeds.trending,
        "new_releases": st.feeds.new_releases.iter().map(dto::release).collect::<Vec<_>>(),
        "mixes": st.feeds.mixes,
        "loading": st.feeds.loading,
    })
}

fn emit_feeds(core: &Core) {
    let v = feeds_json(core);
    core.emit("home", v);
}

/// Keeps what Home may show: no hidden explicit songs, no hidden artists.
fn feed_ok(st: &crate::controller::State, t: &Track) -> bool {
    !(t.maybe_explicit() && st.config.audio.hide_explicit) && !hidden_artist(&st.library, &t.artist)
}

/// "New from artists you follow": releases of the last eight weeks, cached
/// for 12 hours per follow list.
async fn load_new_releases(core: &Arc<Core>) {
    const ARTISTS: usize = 40;
    const DAYS: u32 = 56;
    let scope = scope_of(core);
    let artists: Vec<String> = core.st().library.followed.iter().take(ARTISTS).map(|a| a.name.clone()).collect();
    if artists.is_empty() {
        core.st().feeds.new_releases.clear();
        return;
    }
    if let Some(cached) = load_releases_cache(&scope, &artists, 12 * 3600) {
        core.st().feeds.new_releases = cached;
        return;
    }
    // An older list for the same artists is shown while the fresh one loads.
    if core.st().feeds.new_releases.is_empty() {
        if let Some(stale) = load_releases_cache(&scope, &artists, STALE_FEEDS_S) {
            let mut st = core.st();
            let list: Vec<Release> = stale.into_iter().filter(|r| !hidden_artist(&st.library, &r.artist)).take(20).collect();
            st.feeds.new_releases = list;
            drop(st);
            emit_feeds(core);
        }
    }
    let Some(b) = core.backend() else { return };
    if let Ok(list) = b.new_releases(artists.clone(), DAYS).await {
        save_releases_cache(&scope, &artists, &list);
        let mut st = core.st();
        let shown: Vec<Release> = list.into_iter().filter(|r| !hidden_artist(&st.library, &r.artist)).take(20).collect();
        st.feeds.new_releases = shown;
    }
}

/// Follow / unfollow: only that artist's releases join or leave the row.
pub fn follow_changed(core: &Arc<Core>, name: String, now: bool) {
    const DAYS: u32 = 56;
    if !now {
        let scope = scope_of(core);
        let artists: Vec<String> = core.st().library.followed.iter().take(40).map(|a| a.name.clone()).collect();
        let left = {
            let mut st = core.st();
            st.feeds.new_releases.retain(|r| !r.artist.eq_ignore_ascii_case(&name));
            st.feeds.new_releases.clone()
        };
        save_releases_cache(&scope, &artists, &left);
        emit_feeds(core);
        return;
    }
    let Some(b) = core.backend() else { return };
    let core = core.clone();
    crate::rt::spawn(async move {
        let Ok(found) = b.new_releases(vec![name], DAYS).await else { return };
        let scope = scope_of(&core);
        let artists: Vec<String> = core.st().library.followed.iter().take(40).map(|a| a.name.clone()).collect();
        let list = {
            let mut st = core.st();
            for r in found {
                if !st.feeds.new_releases.iter().any(|x| x.id == r.id) {
                    st.feeds.new_releases.push(r);
                }
            }
            st.feeds.new_releases.sort_by(|a, b| b.released.cmp(&a.released));
            st.feeds.new_releases.truncate(20);
            st.feeds.new_releases.clone()
        };
        save_releases_cache(&scope, &artists, &list);
        emit_feeds(&core);
    });
}

/// Loads "Recommended for you", "Popular right now" and the new releases;
/// every stage that is ready goes to the UI as a `home` event.
pub async fn load_feeds(core: &Arc<Core>) -> Value {
    let scope = scope_of(core);
    // Last session's feeds on screen at once (also before the backend is
    // up); the fresh ones replace them as they arrive.
    {
        let mut st = core.st();
        if st.feeds.trending.is_empty() && st.feeds.recs.is_empty() {
            if let Some((trending, recs)) = load_feeds_cache(&scope, STALE_FEEDS_S) {
                let t: Vec<Track> = trending.into_iter().filter(|t| feed_ok(&st, t)).collect();
                let r: Vec<Track> = recs.into_iter().filter(|t| feed_ok(&st, t)).collect();
                st.feeds.trending = t;
                st.feeds.recs = r;
            }
        }
        if st.feeds.loading {
            drop(st);
            return feeds_json(core);
        }
        st.feeds.loading = true;
    }
    let epoch = core.st().feeds.epoch;
    // A reload (sign-in, sign-out, feedback) came after this load started.
    let stale = |core: &Arc<Core>| core.st().feeds.epoch != epoch;
    emit_feeds(core);
    load_new_releases(core).await;
    let Some(b) = core.backend() else {
        core.st().feeds.loading = false;
        return feeds_json(core);
    };
    emit_feeds(core);
    // Data saver: reuse the last feeds for up to 12 hours.
    if core.st().config.storage.data_saver {
        if let Some((trending, recs)) = load_feeds_cache(&scope, 12 * 3600) {
            {
                let mut st = core.st();
                st.feeds.trending = trending;
                st.feeds.recs = recs;
                st.feeds.loading = false;
            }
            emit_feeds(core);
            return feeds_json(core);
        }
    }
    // Refresh genre/mood tags & optional server collaborative filtering in background.
    crate::taste::enrich_and_sync(core, false).await;
    if stale(core) {
        return feeds_json(core);
    }
    // Seeds: one song for each of the user's top artists, onboarding picks, then liked songs.
    let (seeds, taste, taste_on, recent, feedback, collab_artists) = {
        let st = core.st();
        let taste_on = st.config.taste.enabled;
        let taste = if taste_on {
            glyph_core::taste::Taste::from_library(&st.library)
        } else {
            glyph_core::taste::Taste::default()
        };
        let mut seeds: Vec<Track> = Vec::new();
        let pool: Vec<Track> = st
            .library
            .liked
            .iter()
            .chain(st.library.taste.onboarding_tracks.iter())
            .chain(st.library.recent_tracks())
            .cloned()
            .collect();
        for a in taste.top_artists(4) {
            if let Some(t) = pool.iter().find(|t| glyph_core::taste::credited(&t.artist).contains(&a)) {
                if !seeds.iter().any(|s| s.id == t.id && s.source == t.source) {
                    seeds.push(t.clone());
                }
            }
        }
        for t in st.library.taste.onboarding_tracks.iter().take(2).chain(st.library.liked.iter().take(2)) {
            if !seeds.iter().any(|s| s.id == t.id && s.source == t.source) {
                seeds.push(t.clone());
            }
        }
        seeds.truncate(6);
        let recent: HashSet<String> = st.library.recent_tracks().take(30).map(glyph_core::dedupe::track_key).collect();
        let collab_artists: Vec<(String, f32)> = if taste_on && st.config.taste.server_collab {
            st.library
                .taste
                .collab_cache
                .as_ref()
                .map(|c| {
                    c.recommendations
                        .iter()
                        .take(5)
                        .map(|r| (r.artist.clone(), r.score.clamp(0.0, 2.5)))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        (seeds, taste, taste_on, recent, st.library.recommendation_feedback.clone(), collab_artists)
    };
    // Also fetch top tracks for top onboarding/collaborative artists that lack local seed tracks.
    let mut extra_artists: Vec<String> = collab_artists.iter().take(3).map(|(a, _)| a.clone()).collect();
    if seeds.len() < 3 && taste_on {
        let st = core.st();
        for a in &st.library.taste.onboarding_artists {
            if !extra_artists.iter().any(|x| x.eq_ignore_ascii_case(a)) {
                extra_artists.push(a.clone());
            }
            if extra_artists.len() >= 5 {
                break;
            }
        }
    }
    let lang_str = core.st().config.resolved_language().to_string();
    let extra_fut = futures::future::join_all(extra_artists.iter().map(|a| {
        let b = b.clone();
        let a = a.clone();
        let lang = lang_str.clone();
        async move {
            tokio::time::timeout(Duration::from_secs(4), b.artist_server(&a, &lang))
                .await
                .ok()
                .flatten()
                .map(|s| s.top.into_iter().take(5).collect::<Vec<Track>>())
                .unwrap_or_default()
        }
    }));
    // Popular songs, similar songs of every seed, and collaborative/onboarding artist tracks side by side.
    let (trending, similar, extra_tracks) = futures::join!(
        b.trending(30),
        futures::future::join_all(seeds.iter().map(|s| b.similar(s))),
        extra_fut
    );
    let trending = trending.unwrap_or_default();
    if stale(core) {
        return feeds_json(core);
    }
    {
        let mut st = core.st();
        // Offline: keep what is on screen instead of an empty row.
        if !trending.is_empty() || st.feeds.trending.is_empty() {
            let list: Vec<Track> = trending.iter().filter(|t| feed_ok(&st, t)).take(20).cloned().collect();
            st.feeds.trending = list;
        }
    }
    emit_feeds(core);
    // Songs suggested by several seeds, onboarding picks, and collaborative artists.
    let mut found: Vec<(Track, f32)> = Vec::new();
    for sim in similar {
        for (i, t) in sim.unwrap_or_default().into_iter().take(25).enumerate() {
            let w = 1.0 - i as f32 / 30.0;
            let key = glyph_core::dedupe::track_key(&t);
            match found.iter_mut().find(|(x, _)| glyph_core::dedupe::track_key(x) == key) {
                Some(f) => f.1 += w,
                None => found.push((t, w)),
            }
        }
    }
    for tracks in extra_tracks {
        for (i, t) in tracks.into_iter().enumerate() {
            let w = 0.9 - i as f32 * 0.12;
            let key = glyph_core::dedupe::track_key(&t);
            match found.iter_mut().find(|(x, _)| glyph_core::dedupe::track_key(x) == key) {
                Some(f) => f.1 += w,
                None => found.push((t, w)),
            }
        }
    }
    if stale(core) {
        return feeds_json(core);
    }
    let collab_map: std::collections::HashMap<String, f32> = collab_artists
        .into_iter()
        .map(|(a, s)| (glyph_core::dedupe::normalize_artist(&a), s))
        .collect();
    let bucket = glyph_core::taste::TimeBucket::now();
    let hour = glyph_core::taste::local_hour();
    let date_key = crate::taste::today_key();
    {
        let mut st = core.st();
        let all_candidates: Vec<Track> = found
            .iter()
            .map(|(t, _)| t.clone())
            .chain(trending.iter().cloned())
            .collect();
        let mut v: Vec<(Track, f32)> = found
            .into_iter()
            .filter(|(t, _)| {
                feed_ok(&st, t)
                    && !taste.is_hidden(&t.artist)
                    && !taste.is_disliked_track(t)
                    && !taste.is_liked(t)
                    && !recent.contains(&glyph_core::dedupe::track_key(t))
            })
            .map(|(t, w)| {
                let sc = if taste_on {
                    let cb = glyph_core::taste::credited(&t.artist)
                        .iter()
                        .filter_map(|a| collab_map.get(a))
                        .cloned()
                        .fold(0.0f32, f32::max);
                    taste.score_recommendation(&t, w + cb * 0.65, bucket, false)
                } else {
                    let preference = feedback.get(&t.artist.trim().to_lowercase()).copied().unwrap_or(0);
                    w + preference as f32 * 2.0
                };
                (t, sc)
            })
            .filter(|(_, sc)| sc.is_finite())
            .collect();
        v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        if !(v.is_empty() && !st.feeds.recs.is_empty()) {
            // At most two songs per artist, so it does not become one artist's page.
            let mut per: std::collections::HashMap<String, usize> = Default::default();
            let list: Vec<Track> = v
                .into_iter()
                .filter(|(t, _)| {
                    let n = per.entry(glyph_core::dedupe::normalize_artist(&t.artist)).or_default();
                    *n += 1;
                    *n <= 2
                })
                .map(|(t, _)| t)
                .take(16)
                .collect();
            st.feeds.recs = list;
            save_feeds_cache(&scope, &st.feeds.trending, &st.feeds.recs);
        }
        if taste_on {
            let hide = st.config.audio.hide_explicit;
            st.feeds.mixes = glyph_core::taste::build_daily_mixes(
                &st.library,
                &taste,
                &all_candidates,
                &date_key,
                hour,
                hide,
            );
        } else {
            st.feeds.mixes.clear();
        }
        st.feeds.loading = false;
    }
    emit_feeds(core);
    feeds_json(core)
}

/// Starts loading the feeds again (the backend came up, someone signed in or
/// out, recommendation feedback changed).
pub fn reload_feeds(core: &Arc<Core>) {
    {
        let mut st = core.st();
        st.feeds.recs.clear();
        st.feeds.trending.clear();
        st.feeds.new_releases.clear();
        st.feeds.mixes.clear();
        // A load still running belongs to the old state: let this one start.
        st.feeds.epoch += 1;
        st.feeds.loading = false;
    }
    let core = core.clone();
    crate::rt::spawn(async move {
        load_feeds(&core).await;
    });
}

pub async fn home(core: C<'_>) -> Res<Value> {
    Ok(load_feeds(core).await)
}

// ---- artist pages, albums ----

/// The page as the UI shows it; `info` stays null until the lookup answers.
struct ArtistBuild {
    name: String,
    tracks: Vec<Track>,
    loading: bool,
    info_loading: bool,
    info: Option<Value>,
    stats: Option<glyph_core::catalog::ArtistStats>,
    similar: Vec<String>,
    releases: Vec<Release>,
}

impl ArtistBuild {
    fn json(&self) -> Value {
        let info = match (&self.info, &self.stats) {
            (None, None) => Value::Null,
            (i, s) => {
                let mut v = i.clone().unwrap_or_else(|| json!({ "name": self.name, "bio": Value::Null, "photo": Value::Null, "links": [] }));
                v["fans"] = json!(s.as_ref().map(|s| s.fans));
                v["verified"] = json!(s.as_ref().is_some_and(|s| s.verified));
                v
            }
        };
        json!({
            "name": self.name, "tracks": self.tracks, "loading": self.loading, "info_loading": self.info_loading, "info": info,
            "similar": self.similar, "releases": self.releases.iter().map(dto::release).collect::<Vec<_>>(),
        })
    }
}

/// Artist pages loaded lately, in memory and on disk (`page_cache`).
/// Opening one again shows it at once instead of asking six services again.
fn artist_pages() -> &'static crate::page_cache::PageCache {
    static PAGES: std::sync::LazyLock<crate::page_cache::PageCache> =
        std::sync::LazyLock::new(|| crate::page_cache::PageCache::new("artists", 32));
    &PAGES
}

/// Album track lists loaded lately (title + artist).
fn album_pages() -> &'static crate::page_cache::PageCache {
    static PAGES: std::sync::LazyLock<crate::page_cache::PageCache> =
        std::sync::LazyLock::new(|| crate::page_cache::PageCache::new("albums", 64));
    &PAGES
}

/// An artist page this young is shown without asking again.
const ARTIST_FRESH_S: u64 = 30 * 60;
/// Older ones still show first while the page loads again behind them.
const ARTIST_KEEP_S: u64 = 14 * 24 * 3600;
/// Album track lists rarely change.
const ALBUM_FRESH_S: u64 = 7 * 24 * 3600;

/// Drops the remembered pages from memory under memory pressure.
pub(crate) fn trim_artist_cache() {
    artist_pages().trim();
    album_pages().trim();
}

/// A release from the server's artist page in the app's own shape.
fn server_release(r: glyph_core::server_client::ServerRelease, artist: &str) -> Release {
    let year = r.released.as_deref().and_then(|d| d.get(..4)).and_then(|y| y.parse().ok()).filter(|y| *y > 1900);
    Release {
        id: r.id,
        title: r.title,
        artist: artist.to_string(),
        cover_url: r.cover_url,
        year,
        released: r.released,
        kind: r.record_type.as_deref().and_then(glyph_core::catalog::AlbumKind::parse).unwrap_or(glyph_core::catalog::AlbumKind::Album),
    }
}

/// `quiet`: an older copy of the page shows already; only the finished page
/// replaces it (half-loaded steps would make it jump).
fn artist_event(core: &Core, page: &std::sync::Mutex<ArtistBuild>, quiet: bool) {
    if quiet {
        return;
    }
    let v = page.lock().unwrap_or_else(|e| e.into_inner()).json();
    core.emit("artist", v);
}

/// Opens the artist profile: the songs, the facts and the discography load
/// side by side; every part that is ready goes to the UI as an `artist` event.
/// `quiet`: not remembered in the sidebar (restoring a page).
pub async fn artist(core: C<'_>, name: String, quiet: Option<bool>) -> Res<Value> {
    let core = core.clone();
    let backend = core.backend_or_err();
    let name = glyph_core::dedupe::primary_artist(&name);
    if name.is_empty() {
        return Err("no artist".into());
    }
    // Artist pages show in the sidebar next to the lists.
    if !quiet.unwrap_or(false) {
        {
            let mut st = core.st();
            st.library.push_recent_artist_page(&name, None);
            st.library.record_use(&glyph_core::Library::artist_id(&name), 0.25, now_s());
        }
        core.library_changed();
    }
    let lang = core.st().config.resolved_language().to_string();
    let cache_key = format!("{}|{lang}", name.to_lowercase());
    let cached = artist_pages().get(&cache_key).filter(|(age, _)| *age < ARTIST_KEEP_S);
    if let Some((age, v)) = &cached {
        core.emit("artist", v.clone());
        if *age < ARTIST_FRESH_S {
            return Ok(v.clone());
        }
    }
    let refresh = cached.is_some();
    // Offline or still starting: the older copy is better than nothing.
    let b = match (backend, &cached) {
        (Ok(b), _) => b,
        (Err(_), Some((_, v))) => return Ok(v.clone()),
        (Err(e), None) => return Err(e),
    };
    let page = std::sync::Mutex::new(ArtistBuild { name: name.clone(), tracks: vec![], loading: true, info_loading: true, info: None, stats: None, similar: vec![], releases: vec![] });
    let lock = || page.lock().unwrap_or_else(|e| e.into_inner());
    // The server keeps a whole artist page (popular songs, discography,
    // related artists, fans) for 14 days for every app: one quick request
    // instead of a search and four lookups on the phone. What it lacks is
    // still looked up here.
    let from_server = tokio::time::timeout(Duration::from_secs(4), b.artist_server(&name, &lang)).await.ok().flatten();
    let (mut have_songs, mut have_stats, mut have_similar, mut have_releases) = (false, false, false, false);
    if let Some(s) = from_server {
        let mut p = lock();
        let top: Vec<Track> = s.top.into_iter().filter(|t| glyph_core::dedupe::by_artist(&t.artist, &name)).collect();
        if top.len() >= 5 {
            p.tracks = top;
            p.loading = p.info_loading;
            have_songs = true;
        }
        if let Some(fans) = s.fans {
            p.stats = Some(glyph_core::catalog::ArtistStats { fans, verified: s.verified });
            have_stats = true;
        }
        let similar: Vec<String> = s.similar.into_iter().filter(|n| !n.eq_ignore_ascii_case(&name)).collect();
        if !similar.is_empty() {
            p.similar = similar;
            have_similar = true;
        }
        if !s.releases.is_empty() {
            p.releases = s.releases.into_iter().map(|r| server_release(r, &name)).collect();
            have_releases = true;
        }
        drop(p);
        if have_songs || have_stats || have_similar || have_releases {
            artist_event(&core, &page, refresh);
        }
    }
    // Songs: a normal search for the name, keeping the artist's own songs.
    let songs = async {
        if have_songs {
            return;
        }
        let mut found: Vec<Track> = Vec::new();
        let mut stream = b.search(&name);
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let Ok(Some(ev)) = tokio::time::timeout(left, stream.next()).await else { break };
            match ev {
                SearchEvent::Results { tracks, .. } => {
                    let mut changed = false;
                    for tr in tracks {
                        let dup = found.iter().any(|x| glyph_core::dedupe::normalize_title(&x.title) == glyph_core::dedupe::normalize_title(&tr.title));
                        if glyph_core::dedupe::by_artist(&tr.artist, &name) && !dup {
                            found.push(tr);
                            changed = true;
                        }
                    }
                    if changed {
                        lock().tracks = found.clone();
                        artist_event(&core, &page, refresh);
                    }
                }
                SearchEvent::Done => break,
                _ => {}
            }
        }
        // The search knows no song under this exact name (other scripts,
        // channel names): the songs of the latest releases stand in, so the
        // page is never empty next to its albums.
        if found.is_empty() {
            if let Ok(releases) = b.artist_releases(&name).await {
                for r in releases.iter().take(6) {
                    let Ok(tracks) = b.release_tracks(r).await else { continue };
                    for tr in tracks {
                        let n = glyph_core::dedupe::normalize_title(&tr.title);
                        if !found.iter().any(|x| glyph_core::dedupe::normalize_title(&x.title) == n) {
                            found.push(tr);
                        }
                    }
                    if found.len() >= 10 {
                        break;
                    }
                }
            }
        }
        {
            let mut p = lock();
            if p.tracks.is_empty() {
                p.tracks = found;
            }
            p.loading = p.info_loading;
        }
        artist_event(&core, &page, refresh);
    };
    let stats = async {
        if have_stats {
            return;
        }
        if let Ok(Some(s)) = b.artist_stats(&name).await {
            lock().stats = Some(s);
            artist_event(&core, &page, refresh);
        }
    };
    let similar = async {
        if have_similar {
            return;
        }
        if let Ok(v) = b.similar_artists(&name).await {
            // The artist itself is never its own suggestion.
            lock().similar = v.into_iter().filter(|n| !n.eq_ignore_ascii_case(&name)).collect();
            artist_event(&core, &page, refresh);
        }
    };
    let releases = async {
        if have_releases {
            return;
        }
        if let Ok(v) = b.artist_releases(&name).await {
            lock().releases = v;
            artist_event(&core, &page, refresh);
        }
    };
    let info = async {
        let res = b.artist_info(&name, &lang).await;
        {
            let mut p = lock();
            p.info_loading = false;
            p.loading = false;
            if let Ok(Some(i)) = res {
                let photo = i.image.as_ref().map(|p| crate::host::asset_url(p));
                let banner = i.banner.as_ref().map(|p| crate::host::asset_url(p)).or_else(|| photo.clone());
                let links: Vec<Value> = if i.url.is_empty() { vec![] } else { vec![json!({ "label": "Wikipedia", "url": i.url })] };
                p.info = Some(json!({ "name": name, "bio": i.about, "photo": photo, "banner": banner, "links": links }));
            }
        }
        artist_event(&core, &page, refresh);
    };
    futures::join!(songs, stats, similar, releases, info);
    {
        let mut p = lock();
        p.loading = false;
        p.info_loading = false;
    }
    let v = lock().json();
    // A page without songs (all lookups failed, offline) is not kept, and
    // never replaces an older one that had songs.
    if v["tracks"].as_array().is_some_and(|t| !t.is_empty()) {
        artist_pages().put(&cache_key, &v);
    } else if let Some((_, old)) = cached {
        return Ok(old);
    }
    if refresh {
        core.emit("artist", v.clone());
    }
    Ok(v)
}

/// The artist's picture for tiles, without loading the page. `None`: none known.
pub async fn artist_photo(core: C<'_>, name: String) -> Res<Option<String>> {
    let b = core.backend_or_err()?;
    let lang = core.st().config.resolved_language().to_string();
    let name = glyph_core::dedupe::primary_artist(&name);
    Ok(b.artist_photo(&name, &lang).await.map_err(err)?.map(|p| crate::host::asset_url(&p)))
}

fn verified_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, bool>> {
    static KNOWN: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, bool>>> = std::sync::OnceLock::new();
    KNOWN.get_or_init(Default::default)
}

/// Drops cached artist-verification lookups under memory pressure.
pub(crate) fn trim_verified_cache(aggressive: bool) {
    let mut k = verified_cache().lock().unwrap_or_else(|e| e.into_inner());
    if aggressive || k.len() > 256 {
        k.clear();
        k.shrink_to_fit();
    }
}

/// Is the artist verified? Looked up once per name (with bounded concurrency).
pub async fn artist_verified(core: C<'_>, name: String) -> Res<bool> {
    static SEM: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    let known = verified_cache();
    let who = glyph_core::dedupe::primary_artist(&name);
    let key = who.to_lowercase();
    if key.is_empty() {
        return Ok(false);
    }
    if let Some(v) = known.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(*v);
    }
    let _permit = SEM.get_or_init(|| tokio::sync::Semaphore::new(4)).acquire().await.ok();
    if let Some(v) = known.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(*v);
    }
    let b = core.backend_or_err()?;
    let v = matches!(b.artist_stats(&who).await, Ok(Some(s)) if s.verified);
    let mut k = known.lock().unwrap_or_else(|e| e.into_inner());
    if k.len() > 1000 {
        k.clear();
    }
    k.insert(key, v);
    Ok(v)
}

/// The songs of an album: from the catalog, else a search for "artist album".
/// The songs of an album; kept a week (`album_pages`), and an older copy is
/// used when the lookup finds nothing (offline, a service down).
pub async fn album(core: C<'_>, title: String, artist: String) -> Res<Vec<Track>> {
    let artist = glyph_core::dedupe::primary_artist(&artist);
    let key = format!("{}|{}", title.trim().to_lowercase(), artist.trim().to_lowercase());
    let cached: Option<(u64, Vec<Track>)> = album_pages()
        .get(&key)
        .and_then(|(age, v)| Some((age, serde_json::from_value::<Vec<Track>>(v).ok()?)))
        .filter(|(_, t)| !t.is_empty());
    if let Some((age, tracks)) = &cached {
        if *age < ALBUM_FRESH_S {
            return Ok(tracks.clone());
        }
    }
    match album_lookup(core, &title, &artist).await {
        Ok(found) if !found.is_empty() => {
            album_pages().put(&key, &json!(found));
            Ok(found)
        }
        other => match cached {
            Some((_, tracks)) => Ok(tracks),
            None => other,
        },
    }
}

async fn album_lookup(core: C<'_>, title: &str, artist: &str) -> Res<Vec<Track>> {
    let b = core.backend_or_err()?;
    let (title, artist) = (title.to_string(), artist.to_string());
    // The server's cached discography first; the catalog itself when it has none.
    let lang = core.st().config.resolved_language().to_string();
    let from_server: Vec<Release> = tokio::time::timeout(Duration::from_secs(4), b.artist_server(&artist, &lang))
        .await
        .ok()
        .flatten()
        .map(|s| s.releases.into_iter().map(|r| server_release(r, &artist)).collect())
        .unwrap_or_default();
    let releases = if from_server.iter().any(|r| r.title.eq_ignore_ascii_case(&title)) {
        from_server
    } else {
        b.artist_releases(&artist).await.unwrap_or_default()
    };
    if let Some(r) = releases.iter().find(|r| r.title.eq_ignore_ascii_case(&title)) {
        if let Ok(tracks) = b.release_tracks(r).await {
            if !tracks.is_empty() {
                return Ok(tracks);
            }
        }
    }
    let mut stream = b.search(&format!("{artist} {title}"));
    let mut found: Vec<Track> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok(Some(ev)) = tokio::time::timeout(left, stream.next()).await else { break };
        match ev {
            SearchEvent::Results { tracks, .. } => {
                for t in tracks {
                    if t.album.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(&title)) && glyph_core::dedupe::by_artist(&t.artist, &artist) {
                        let n = glyph_core::dedupe::normalize_title(&t.title);
                        if !found.iter().any(|x| glyph_core::dedupe::normalize_title(&x.title) == n) {
                            found.push(t);
                        }
                    }
                }
            }
            SearchEvent::Done => break,
            _ => {}
        }
    }
    Ok(found)
}

pub async fn lyrics(core: C<'_>, track: Track) -> Res<Option<Value>> {
    let b = core.backend_or_err()?;
    let offset = core.st().library.lyrics_offset(&track);
    Ok(b.lyrics(&track).await.map_err(err)?.map(|l| dto::lyrics(&l, offset)))
}

pub async fn cover_path(core: C<'_>, track: Track) -> Res<Option<String>> {
    let b = core.backend_or_err()?;
    let Some(res) = b.cover_with_meta(&track).await.map_err(err)? else {
        return Ok(None);
    };
    if res.album.is_some() || res.cover_url.is_some() {
        let changed = {
            let mut st = core.st();
            st.library.backfill_track_meta(&track, res.album.as_deref(), res.cover_url.as_deref())
        };
        if changed {
            core.library_changed_soon();
        }
    }
    Ok(Some(res.path.to_string_lossy().into_owned()))
}

pub async fn track_album(core: C<'_>, track: Track) -> Res<Option<String>> {
    let b = core.backend_or_err()?;
    let Some((cover_url, album)) = b.resolve_track_meta(&track).await.map_err(err)? else {
        return Ok(None);
    };
    let changed = {
        let mut st = core.st();
        st.library.backfill_track_meta(&track, album.as_deref(), cover_url.as_deref())
    };
    if changed {
        core.library_changed_soon();
    }
    Ok(album)
}

// ---- settings, system ----

/// Applies `cfg` to in-memory state, player, theme icon, disk, UI events, and backend sync.
pub(crate) async fn commit_config(core: &Arc<Core>, mut cfg: Config, preserve_player_controls: bool) -> Res {
    let (stats_was, shuffle_was, taste_was) = {
        let mut st = core.st();
        let was = (st.config.stats.enabled, st.config.playback.shuffle, st.config.taste.clone());
        if preserve_player_controls {
            // Volume, shuffle and repeat are the player's own: the UI's copy may be older.
            cfg.player.volume = st.config.player.volume;
            cfg.player.shuffle = st.config.player.shuffle;
            cfg.player.repeat = st.config.player.repeat.clone();
        }
        st.config = cfg.clone();
        was
    };
    if shuffle_was != cfg.playback.shuffle {
        core.apply_shuffle_style();
    }
    if stats_was != cfg.stats.enabled {
        crate::discover::on_stats_toggle(core, cfg.stats.enabled);
    }
    if taste_was != cfg.taste {
        crate::taste::on_taste_config_changed(core, &taste_was, &cfg.taste);
    }
    crate::settings::apply_to_player(core, &cfg);
    core.config_changed();
    crate::theme_icon::sync_host_icon(core);
    if let Some(b) = core.backend() {
        b.set_config(cfg).await.map_err(err)?;
    }
    Ok(())
}

pub async fn set_config(core: C<'_>, config: Value) -> Res {
    let cfg: Config = serde_json::from_value(config).map_err(err)?;
    commit_config(core, cfg, true).await
}

pub fn output_devices() -> Vec<String> {
    glyph_audio::PlayerController::output_devices()
}

pub fn open_url(url: String) -> Res {
    if !(url.starts_with("https://") || url.starts_with("http://") || url.starts_with("mailto:") || url.starts_with("sms:")) {
        return Err("unsupported link".into());
    }
    crate::host::get().open_url(&url)
}

/// Hands `text` to the system share sheet (phone share menu, "More").
pub fn share_system(text: String, title: Option<String>) -> Res {
    crate::host::get().share_text(&text, title.as_deref())
}

/// Puts `text` on the clipboard (phone share menu, "Copy link").
pub fn copy_text(text: String) -> Res {
    crate::host::get().write_clipboard(&text)
}

pub fn reveal_data_dir() -> Res {
    crate::host::get().open_path(&glyph_core::paths::data_dir())
}

pub fn downloaded_bytes(core: C, tracks: Vec<Track>) -> u64 {
    core.backend().map_or(0, |b| b.downloaded_bytes(&tracks))
}

// ---- mcp ----

pub async fn mcp_server_token(core: C<'_>, name: Option<String>) -> Res {
    crate::mcp_tools::request_server_token(core, name).await;
    Ok(())
}

pub async fn mcp_sessions(core: C<'_>) -> Res<Vec<glyph_protocol::mcp::Session>> {
    crate::mcp_tools::sessions(core).await
}

pub async fn mcp_rename_session(core: C<'_>, id: String, name: String) -> Res<glyph_protocol::mcp::Session> {
    crate::mcp_tools::rename_session(core, id, name).await
}

pub async fn mcp_revoke_session(core: C<'_>, id: String) -> Res {
    crate::mcp_tools::revoke_session(core, id).await
}

pub async fn mcp_claim_oauth(core: C<'_>, id: String) -> Res<glyph_protocol::mcp::Approval> {
    crate::mcp_tools::claim_oauth(core, id).await
}
