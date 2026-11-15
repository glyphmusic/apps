//! Shapes the Svelte UI expects (`glyph-ui/src/lib/api/types.ts`), built
//! from glyph-core / glyph-audio values. Field names stay snake_case.

use glyph_audio::{PlayerState, Status};
use glyph_core::{Library, Lyrics, Queue, RecentItem, RepeatMode};
use glyph_protocol::{ProviderId, Track};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize)]
pub struct Player {
    pub status: &'static str,
    pub current: Option<Track>,
    /// Seconds.
    pub position: f64,
    pub duration: Option<f64>,
    pub volume: f32,
    pub muted: bool,
    pub shuffle: bool,
    pub repeat: &'static str,
    pub source: Option<ProviderId>,
    pub preview: bool,
    pub error: Option<String>,
    pub loop_a: Option<f64>,
    pub loop_b: Option<f64>,
}

pub struct PlayerInput<'a> {
    pub state: Option<&'a PlayerState>,
    pub current: Option<&'a Track>,
    pub resolving: bool,
    pub muted: bool,
    pub volume: f32,
    pub queue: &'a Queue,
    pub error: Option<&'a String>,
    pub loop_ab: Option<(f64, f64)>,
    pub restored_pos: Option<std::time::Duration>,
    pub restored_duration: Option<std::time::Duration>,
    pub resume_at: Option<std::time::Duration>,
}

pub fn repeat_name(r: RepeatMode) -> &'static str {
    match r {
        RepeatMode::Off => "off",
        RepeatMode::All => "all",
        RepeatMode::One => "one",
    }
}

pub fn repeat_from(s: &str) -> RepeatMode {
    match s {
        "all" => RepeatMode::All,
        "one" => RepeatMode::One,
        _ => RepeatMode::Off,
    }
}

pub fn player(i: PlayerInput) -> Player {
    let status = if i.resolving {
        "loading"
    } else if i.error.is_some() {
        "error"
    } else {
        match i.state.map(|s| s.status) {
            Some(Status::Loading) => "loading",
            Some(Status::Playing) => "playing",
            Some(Status::Paused) | Some(Status::Ended) => "paused",
            Some(Status::Idle) | None => if i.current.is_some() { "paused" } else { "idle" },
        }
    };
    let duration = i
        .state
        .and_then(|s| s.duration)
        .or(i.restored_duration)
        .map(|d| d.as_secs_f64())
        .or_else(|| i.current.and_then(|t| t.duration_ms).map(|ms| ms as f64 / 1000.0));
    Player {
        status,
        current: i.current.cloned(),
        position: if i.resolving {
            i.resume_at.map_or(0.0, |d| d.as_secs_f64())
        } else if let Some(pos) = i.restored_pos {
            pos.as_secs_f64()
        } else {
            i.state.map_or(0.0, |s| s.position.as_secs_f64())
        },
        duration,
        volume: i.volume,
        muted: i.muted,
        shuffle: i.queue.shuffle(),
        repeat: repeat_name(i.queue.repeat()),
        source: i.state.and_then(|s| s.provider).or_else(|| i.current.map(|t| t.source)),
        preview: i.state.is_some_and(|s| s.is_preview),
        error: i.error.cloned(),
        loop_a: i.loop_ab.map(|(a, _)| a),
        loop_b: i.loop_ab.map(|(_, b)| b),
    }
}

/// `{ items, index, user, context }`: the played songs, the current one and
/// what comes next, in one list; `index` points at the current song.
pub fn queue(q: &Queue, context: Option<&String>) -> Value {
    let history: Vec<Track> = q.history().into_iter().cloned().collect();
    let current = q.current().cloned();
    let upcoming: Vec<Track> = q.upcoming().into_iter().cloned().collect();
    let index = current.as_ref().map(|_| history.len());
    let mut items = history;
    items.extend(current);
    items.extend(upcoming);
    json!({ "items": items, "index": index, "user": q.manual_len(), "context": context })
}

/// Index into the UI's queue list -> index into `Queue::upcoming()`.
pub fn upcoming_index(q: &Queue, ui_index: usize) -> Option<usize> {
    let first = q.history().len() + usize::from(q.current().is_some());
    ui_index.checked_sub(first)
}

pub fn library(lib: &Library, downloads: &[Track]) -> Value {
    let recent: Vec<Value> = lib
        .recent
        .iter()
        .map(|r| match r {
            RecentItem::Song { track } => json!({ "kind": "song", "track": track }),
            RecentItem::Artist { name, track } => json!({ "kind": "artist", "name": name, "track": track }),
        })
        .collect();
    let trash: Vec<Value> = lib
        .playlist_trash
        .iter()
        .map(|(id, p)| json!({ "id": id, "name": p.name }))
        .collect();
    // Own playlist covers are files; the webview loads them via the asset protocol.
    let playlists: Vec<Value> = lib
        .playlists
        .iter()
        .map(|p| {
            let mut v = json!(p);
            if let Some(c) = p.cover.as_deref().filter(|c| !c.starts_with("http")) {
                v["cover"] = json!(crate::host::asset_url(std::path::Path::new(c)));
            }
            v
        })
        .collect();
    let mut v = json!({
        "liked": lib.liked,
        "recent": recent,
        "playlists": playlists,
        "saved_albums": lib.saved_albums,
        "pinned": lib.pinned,
        "recent_lists": lib.recent_lists,
        "downloads": downloads,
        "search_history": lib.search_items,
        "playlist_trash": trash,
    });
    v["liked_auto_download"] = json!(lib.liked_auto_download);
    // Older saved versions of own playlists (newest first), not the current state.
    let mut versions: Vec<(&String, &glyph_core::library::PlaylistRevision)> = lib
        .playlist_versions
        .iter()
        .filter(|(_, r)| lib.playlist(&r.playlist.id) != Some(&r.playlist))
        .collect();
    versions.sort_by_key(|(_, r)| std::cmp::Reverse(r.saved_at));
    v["playlist_versions"] = json!(versions
        .iter()
        .map(|(k, r)| json!({ "revision": k, "id": r.playlist.id, "saved_at": date_text(r.saved_at) }))
        .collect::<Vec<_>>());
    // Sidebar: artist pages opened lately, and how much each entry is used
    // (listening counts most, ties go by recency).
    v["recent_artists"] = json!(lib.recent_artists.iter().map(|a| json!({ "name": a.name, "track": a.track })).collect::<Vec<_>>());
    let now = glyph_core::paths::now_unix();
    let mut weights = serde_json::Map::new();
    for k in lib.recent_lists.iter().chain(lib.pinned.iter()) {
        let id = Library::list_id(k);
        weights.insert(id, json!(lib.list_weight(k, now)));
    }
    for a in &lib.recent_artists {
        let id = Library::artist_id(&a.name);
        weights.insert(id.clone(), json!(lib.use_weight(&id, now)));
    }
    v["sidebar_weights"] = Value::Object(weights);
    v["followed"] = json!(lib.followed.iter().map(|f| f.name.as_str()).collect::<Vec<_>>());
    v["hidden_sidebar"] = json!(lib.sidebar_hidden);
    v["hidden_artists"] = json!(lib.hidden_artists);
    v
}

pub fn lyrics(l: &Lyrics, offset_ms: i64) -> Value {
    let synced = !l.synced.is_empty();
    let lines: Vec<Value> = if synced {
        l.synced.iter().map(|x| json!({ "time_ms": x.time_ms, "text": x.text })).collect()
    } else {
        l.plain
            .as_deref()
            .unwrap_or_default()
            .lines()
            .map(|t| json!({ "time_ms": Value::Null, "text": t }))
            .collect()
    };
    json!({ "synced": synced, "lines": lines, "score": l.match_score, "source": l.provider, "offset_ms": offset_ms, "instrumental": l.instrumental })
}

pub fn release(r: &glyph_core::providers::deezer::Release) -> Value {
    let kind = match r.kind {
        glyph_core::catalog::AlbumKind::Album => "album",
        glyph_core::catalog::AlbumKind::Ep => "ep",
        glyph_core::catalog::AlbumKind::Single => "single",
        glyph_core::catalog::AlbumKind::Compilation => "compilation",
    };
    json!({ "id": r.id, "title": r.title, "artist": r.artist, "kind": kind, "year": r.year, "date": r.released, "cover_url": r.cover_url })
}

/// "2026-11-15 14:30" (UTC) for a Unix time.
fn date_text(t: u64) -> String {
    let (days, rem) = ((t / 86_400) as i64, t % 86_400);
    // Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rem / 3600, rem % 3600 / 60)
}
