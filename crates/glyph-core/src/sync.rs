//! Library sync with a Glyph account (see `docs/ACCOUNTS.md`).
//!
//! The library is not instrumented: every round takes a [`Snapshot`] of the
//! synced parts, compares it with what was last agreed with the server
//! ([`SyncState::known`], a hash per item) and pushes the difference. Remote
//! items are applied to the [`Library`] and the post-apply snapshot becomes the
//! new agreement, so a round never echoes its own changes back.
//!
//! Synced: liked songs, playlists (incl. their picture), followed and hidden
//! artists, saved albums, recent searches and which songs are downloaded.

use crate::library::{FollowedArtist, Library, ListKey, Playlist, SearchItem, SEARCH_HISTORY_LIMIT};
use crate::paths;
use glyph_protocol::{sync_kind as kind, ProviderId, SyncItem, Track};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// `(kind, key)`
pub type Key = (String, String);

const DELETED: &str = "-";
const RECENT: &str = "recent";
const ACTIVITY: &str = "song_activity";
const PREFERENCE: &str = "library_pref";

fn provider_str(p: ProviderId) -> String {
    serde_json::to_value(p).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// Stable id of a song across devices.
pub fn track_id(t: &Track) -> String {
    format!("{}:{}", provider_str(t.source), t.id)
}

fn hash(v: &Value) -> String {
    if let Some(obj) = v.as_object() {
        if obj.contains_key("base") {
            let mut clean = obj.clone();
            clean.remove("base");
            let d = Sha256::digest(Value::Object(clean).to_string().as_bytes());
            return d[..12].iter().map(|b| format!("{b:02x}")).collect();
        }
    }
    let d = Sha256::digest(v.to_string().as_bytes());
    d[..12].iter().map(|b| format!("{b:02x}")).collect()
}

fn playlist_track_ids(v: &Value) -> Vec<String> {
    serde_json::from_value::<Vec<Track>>(v["tracks"].clone())
        .unwrap_or_default()
        .iter()
        .map(track_id)
        .collect()
}

fn merge_playlist_tracks(
    local_base: Option<&[String]>,
    remote_base: Option<&[String]>,
    local: &[Track],
    remote: Vec<Track>,
) -> Vec<Track> {
    let local_ids: Vec<String> = local.iter().map(track_id).collect();
    let remote_ids: Vec<String> = remote.iter().map(track_id).collect();
    if local_ids == remote_ids {
        return remote;
    }
    if local_base.is_some_and(|lb| lb == local_ids.as_slice())
        && remote_base.is_none_or(|rb| rb == local_ids.as_slice())
    {
        return remote;
    }
    let remote_set: BTreeSet<&str> = remote_ids.iter().map(String::as_str).collect();
    let local_set: BTreeSet<&str> = local_ids.iter().map(String::as_str).collect();
    let remote_removed: BTreeSet<&str> = remote_base
        .unwrap_or(&[])
        .iter()
        .map(String::as_str)
        .filter(|id| !remote_set.contains(*id))
        .collect();
    let local_removed: BTreeSet<&str> = local_base
        .unwrap_or(&[])
        .iter()
        .map(String::as_str)
        .filter(|id| !local_set.contains(*id))
        .collect();
    let mut out: Vec<Track> = remote
        .into_iter()
        .filter(|t| !local_removed.contains(track_id(t).as_str()))
        .collect();
    for t in local {
        let id = track_id(t);
        if !remote_removed.contains(id.as_str())
            && !out.iter().any(|x| track_id(x) == id || crate::library::same_entry(x, t))
        {
            out.push(t.clone());
        }
    }
    out
}

fn sha_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Folder for playlist pictures that came from another device.
pub fn synced_covers_dir() -> PathBuf {
    paths::data_dir().join("synced_covers")
}

/// File extension for image bytes (by magic number).
pub fn image_ext(b: &[u8]) -> &'static str {
    if b.starts_with(b"\x89PNG") {
        "png"
    } else if b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        "webp"
    } else if b.starts_with(b"GIF8") {
        "gif"
    } else {
        "jpg"
    }
}

/// Local picture for a cover hash, if it was already fetched.
pub fn synced_cover(sha: &str) -> Option<PathBuf> {
    ["jpg", "png", "webp", "gif"].iter().map(|e| synced_covers_dir().join(format!("{sha}.{e}"))).find(|p| p.exists())
}

/// The synced parts of the library, in push order (oldest first for ordered lists).
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub items: Vec<(Key, Value)>,
    /// Playlist pictures by SHA-256, for uploading.
    pub cover_files: BTreeMap<String, PathBuf>,
}

impl Snapshot {
    fn map(&self) -> BTreeMap<&Key, &Value> {
        self.items.iter().map(|(k, v)| (k, v)).collect()
    }
}

fn k(kind: &str, key: impl Into<String>) -> Key {
    (kind.to_string(), key.into())
}

fn search_key(i: &SearchItem) -> String {
    match i {
        SearchItem::Song { track } => format!("song:{}", track_id(track)),
        SearchItem::Artist { name, .. } => format!("artist:{}", name.trim().to_lowercase()),
    }
}

fn album_key(title: &str, artist: &str) -> String {
    format!("{}|{}", title.trim().to_lowercase(), artist.trim().to_lowercase())
}

/// Everything that syncs. `downloads` are the songs this device should have
/// offline (downloaded or still waiting).
pub fn snapshot(lib: &Library, downloads: &[Track]) -> Snapshot {
    let mut s = Snapshot::default();
    let mut push = |key: Key, v: Value| s.items.push((key, v));
    for (key, revision) in &lib.playlist_versions {
        if !revision.playlist.local_only { push(k("playlist_version", key), serde_json::to_value(revision).unwrap_or_default()); }
    }
    for (key, playlist) in &lib.playlist_trash {
        if !playlist.local_only { push(k("playlist_trash", key), serde_json::to_value(playlist).unwrap_or_default()); }
    }
    for (artist, feedback) in &lib.recommendation_feedback {
        push(k("recommend_feedback", artist), serde_json::json!(feedback));
    }
    for t in lib.liked.iter().rev() {
        push(k(kind::LIKED, track_id(t)), serde_json::to_value(t).unwrap_or_default());
    }
    for p in lib.playlists.iter().filter(|p| !p.local_only) {
        let mut v = serde_json::json!({ "name": p.name, "tracks": p.tracks, "origin": p.origin, "shared_by": p.shared_by });
        if let Some(membership) = lib.playlist_membership.get(&p.id) { v["membership"] = serde_json::to_value(membership).unwrap_or_default(); }
        if let Some(bytes) = p.cover.as_deref().and_then(|c| std::fs::read(c).ok()) {
            let sha = sha_hex(&bytes);
            v["cover"] = Value::String(sha.clone());
            s.cover_files.insert(sha, PathBuf::from(p.cover.clone().unwrap_or_default()));
        }
        s.items.push((k(kind::PLAYLIST, p.id.clone()), v));
    }
    let mut push = |key: Key, v: Value| s.items.push((key, v));
    for a in &lib.followed {
        push(k(kind::FOLLOWED, a.name.trim().to_lowercase()), serde_json::to_value(a).unwrap_or_default());
    }
    for a in &lib.hidden_artists {
        push(k(kind::HIDDEN, a.trim().to_lowercase()), serde_json::json!({ "name": a }));
    }
    for l in lib.saved_albums.iter().rev() {
        if let ListKey::Album { title, artist, tracks } = l {
            push(k(kind::ALBUM, album_key(title, artist)), serde_json::json!({ "title": title, "artist": artist, "tracks": tracks }));
        }
    }
    for i in lib.search_items.iter().rev() {
        push(k(kind::SEARCH, search_key(i)), serde_json::to_value(i).unwrap_or_default());
    }
    for t in downloads.iter().rev() {
        push(k(kind::DOWNLOAD, track_id(t)), serde_json::to_value(t).unwrap_or_default());
    }
    // Lists that download new songs by themselves (absent = off).
    if lib.liked_auto_download {
        push(k(kind::AUTO_DOWNLOAD, "liked"), serde_json::json!({ "on": true }));
    }
    for p in lib.playlists.iter().filter(|p| p.auto_download && !p.local_only) {
        push(k(kind::AUTO_DOWNLOAD, p.id.clone()), serde_json::json!({ "on": true }));
    }
    // Listening statistics (opt-in): this device's months.
    for (key, v) in lib.stats.sync_items(&crate::server_client::install_id()) {
        push(k(crate::stats::SYNC_KIND, key), v);
    }
    for (key, month) in &lib.guest_stats_imported {
        push(k("guest_stats", key), serde_json::to_value(month).unwrap_or_default());
    }
    for item in lib.recent.iter().rev() {
        let key = match item {
            crate::library::RecentItem::Song { track } => format!("song:{}", track_id(track)),
            crate::library::RecentItem::Artist { name, .. } => format!("artist:{}", name.trim().to_lowercase()),
        };
        push(k(RECENT, key), serde_json::to_value(item).unwrap_or_default());
    }
    for (id, activity) in &lib.song_activity {
        push(k(ACTIVITY, id), serde_json::to_value(activity).unwrap_or_default());
    }
    for (name, value) in [
        ("pinned", serde_json::to_value(&lib.pinned)),
        ("sidebar_hidden", serde_json::to_value(&lib.sidebar_hidden)),
        ("lyrics_offsets", serde_json::to_value(&lib.lyrics_offsets)),
        ("recent_lists", serde_json::to_value(&lib.recent_lists)),
        ("recent_artists", serde_json::to_value(&lib.recent_artists)),
        ("list_activity", serde_json::to_value(&lib.list_activity)),
        ("artist_plays", serde_json::to_value(&lib.artist_plays)),
        ("artist_skips", serde_json::to_value(&lib.artist_skips)),
        ("guest_imports", serde_json::to_value(&lib.guest_imports)),
    ] {
        let value = value.unwrap_or_default();
        if value.as_array().is_some_and(|v| !v.is_empty()) || value.as_object().is_some_and(|v| !v.is_empty()) {
            push(k(PREFERENCE, name), value);
        }
    }
    s
}

/// What this device last agreed on with the account, persisted next to the library.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncState {
    /// Durable unacknowledged writes. Retry the same timestamp after a restart.
    #[serde(default)]
    pub outbox: BTreeMap<String, SyncItem>,
    #[serde(skip)]
    pub storage_path: Option<PathBuf>,
    pub account_id: String,
    pub last_rev: u64,
    /// `kind \u{1f} key` -> value hash (`-` = deleted).
    #[serde(default)]
    pub known: BTreeMap<String, String>,
    /// Last agreed track ids (`track_id`) per playlist id, for 3-way merging
    /// concurrent playlist edits.
    #[serde(default)]
    pub known_playlists: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub known_playlist_revs: BTreeMap<String, u64>,
    /// Songs another device downloaded that this one has not fetched yet.
    #[serde(default)]
    pub pending_downloads: BTreeMap<String, Track>,
    /// Cover hashes already on the server.
    #[serde(default)]
    pub uploaded_covers: BTreeSet<String>,
    /// Encrypted API keys from another device this one cannot open yet (no
    /// or an outdated password key); opened by `account_unlock_keys`.
    #[serde(default)]
    pub locked_secret: Option<Value>,
    /// A full pull finished once; only then are local settings pushed.
    #[serde(default)]
    pub pulled_once: bool,
}

fn joined(key: &Key) -> String {
    format!("{}\u{1f}{}", key.0, key.1)
}

fn split(s: &str) -> Option<Key> {
    s.split_once('\u{1f}').map(|(a, b)| (a.to_string(), b.to_string()))
}

impl SyncState {
    pub fn path() -> PathBuf {
        crate::account::library_path().map(|p| p.with_file_name("sync_state.json")).unwrap_or_else(|| paths::data_dir().join("sync_state.json"))
    }

    /// State for `account_id`; a different account (or none) starts fresh.
    pub fn load(account_id: &str) -> Self {
        Self::load_from(&Self::path(), account_id)
    }

    pub fn load_from(path: &Path, account_id: &str) -> Self {
        let mut state = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<SyncState>(&b).ok())
            .filter(|s| s.account_id == account_id)
            .unwrap_or_else(|| SyncState { account_id: account_id.into(), ..Default::default() });
        state.storage_path = Some(path.to_path_buf());
        state
    }

    pub fn save(&self) -> anyhow::Result<()> {
        paths::write_json(&self.storage_path.clone().unwrap_or_else(Self::path), self)
    }

    /// Songs to keep offline: downloaded here plus the ones still waiting.
    pub fn wanted_downloads(&self, downloaded: &[Track]) -> Vec<Track> {
        let have: BTreeSet<String> = downloaded.iter().map(track_id).collect();
        let mut v = downloaded.to_vec();
        v.extend(self.pending_downloads.iter().filter(|(id, _)| !have.contains(*id)).map(|(_, t)| t.clone()));
        v
    }

    /// Items to push: new or changed since the last agreement, plus tombstones
    /// for agreed items that are gone.
    pub fn changes(&self, snap: &Snapshot, now: u64) -> Vec<SyncItem> {
        let mut out = Vec::new();
        let map = snap.map();
        for (key, v) in &snap.items {
            if self.known.get(&joined(key)).map(String::as_str) != Some(hash(v).as_str()) {
                let mut value = v.clone();
                if key.0 == kind::PLAYLIST {
                    if let Some(base) = self.known_playlists.get(&key.1) {
                        if let Some(obj) = value.as_object_mut() {
                            obj.insert("base".into(), serde_json::to_value(base).unwrap_or_default());
                        }
                    }
                }
                out.push(SyncItem { kind: key.0.clone(), key: key.1.clone(), value: Some(value), updated_at: now, deleted: false, rev: if key.0 == kind::PLAYLIST { self.known_playlist_revs.get(&key.1).copied().unwrap_or(0) } else { 0 } });
            }
        }
        for (j, h) in &self.known {
            let Some(key) = split(j) else { continue };
            if h != DELETED && !map.contains_key(&key) {
                let rev = if key.0 == kind::PLAYLIST { self.known_playlist_revs.get(&key.1).copied().unwrap_or(0) } else { 0 };
                out.push(SyncItem { kind: key.0, key: key.1, value: None, updated_at: now, deleted: true, rev });
            }
        }
        out
    }

    pub fn enqueue(&mut self, snap: &Snapshot, now: u64) -> Vec<SyncItem> {
        let changes = self.changes(snap, now);
        let wanted: BTreeSet<String> = changes.iter().map(|i| joined(&(i.kind.clone(), i.key.clone()))).collect();
        self.outbox.retain(|key, _| wanted.contains(key));
        for item in changes {
            let key = joined(&(item.kind.clone(), item.key.clone()));
            let unchanged = self.outbox.get(&key).is_some_and(|old| old.value == item.value && old.deleted == item.deleted && old.rev == item.rev);
            if !unchanged { self.outbox.insert(key, item); }
        }
        self.outbox.values().cloned().collect()
    }

    /// The server accepted these pushed items.
    pub fn mark_pushed(&mut self, items: &[SyncItem]) {
        for i in items {
            self.outbox.remove(&joined(&(i.kind.clone(), i.key.clone())));
            let h = match (&i.value, i.deleted) {
                (Some(v), false) => hash(v),
                _ => DELETED.into(),
            };
            self.known.insert(joined(&(i.kind.clone(), i.key.clone())), h);
            if i.kind == kind::PLAYLIST {
                match (&i.value, i.deleted) {
                    (Some(v), false) => {
                        self.known_playlists.insert(i.key.clone(), playlist_track_ids(v));
                    }
                    _ => {
                        self.known_playlists.remove(&i.key);
                    }
                }
            }
        }
    }

    /// Record `value` (None = deleted) as agreed with the server for `key`.
    pub fn agree(&mut self, key: &Key, value: Option<&Value>) {
        self.known.insert(joined(key), value.map(hash).unwrap_or_else(|| DELETED.into()));
        if key.0 == kind::PLAYLIST {
            match value {
                Some(v) => {
                    self.known_playlists.insert(key.1.clone(), playlist_track_ids(v));
                }
                None => {
                    self.known_playlists.remove(&key.1);
                }
            }
        }
    }

    /// Forget the agreement for `key`, so a missing local item is not pushed
    /// as a deletion.
    pub fn forget(&mut self, key: &Key) {
        self.known.remove(&joined(key));
        if key.0 == kind::PLAYLIST {
            self.known_playlists.remove(&key.1);
        }
    }

    /// Start over (server reset or a different account): keep nothing agreed.
    pub fn reset(&mut self) {
        self.last_rev = 0;
        self.known.clear();
        self.known_playlists.clear();
        self.known_playlist_revs.clear();
        self.uploaded_covers.clear();
    }
}

/// Side effects of applying remote items the library cannot do itself.
#[derive(Debug, Default, PartialEq)]
pub struct Applied {
    pub library_changed: bool,
    /// Fetch these for offline use.
    pub download: Vec<Track>,
    /// Delete these local files.
    pub remove_downloads: Vec<Track>,
}

fn from<T: serde::de::DeserializeOwned>(v: &Option<Value>) -> Option<T> {
    v.as_ref().and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// Apply remote items (in rev order) and record the result as agreed.
pub fn apply(lib: &mut Library, state: &mut SyncState, items: &[SyncItem], downloaded: &[Track]) -> Applied {
    let mut out = Applied::default();
    let mut touched: Vec<Key> = Vec::new();
    let mut unpushed_playlists: BTreeMap<Key, (String, Vec<String>)> = BTreeMap::new();
    // Auto-download switches go last: their playlist may arrive in the
    // same batch.
    let (switches, items): (Vec<&SyncItem>, Vec<&SyncItem>) = items.iter().partition(|i| i.kind == kind::AUTO_DOWNLOAD);
    for i in items.into_iter().chain(switches) {
        if i.kind == kind::PLAYLIST { state.known_playlist_revs.insert(i.key.clone(), i.rev); }
        let gone = i.deleted || i.value.is_none();
        match i.kind.as_str() {
            "playlist_version" => {
                if let Some(v) = from::<crate::library::PlaylistRevision>(&i.value).filter(|v| !v.playlist.local_only) {
                    lib.playlist_versions.entry(i.key.clone()).or_insert(v);
                }
            }
            "playlist_trash" => {
                if gone { lib.playlist_trash.remove(&i.key); }
                else if let Some(p) = from::<Playlist>(&i.value).filter(|p| !p.local_only && p.id == i.key) { lib.playlist_trash.insert(i.key.clone(), p); }
            }
            "recommend_feedback" => {
                if gone { lib.recommendation_feedback.remove(&i.key); }
                else if let Some(v) = from::<i8>(&i.value).filter(|v| (-2..=1).contains(v)) { lib.recommendation_feedback.insert(i.key.clone(), v); }
            }
            "guest_stats" => {
                if !gone {
                    if let Some(month) = from::<crate::stats::MonthStats>(&i.value) {
                        lib.guest_stats_imported.entry(i.key.clone()).or_default().merge_max(&month);
                    }
                }
            }
            RECENT => {
                use crate::library::RecentItem;
                lib.recent.retain(|item| {
                    let key = match item {
                        RecentItem::Song { track } => format!("song:{}", track_id(track)),
                        RecentItem::Artist { name, .. } => format!("artist:{}", name.trim().to_lowercase()),
                    };
                    key != i.key
                });
                if !gone {
                    if let Some(item) = from::<RecentItem>(&i.value) { lib.recent.insert(0, item); }
                }
                lib.recent.truncate(crate::library::RECENT_LIMIT);
            }
            ACTIVITY => {
                if gone { lib.song_activity.remove(&i.key); }
                else if let Some(activity) = from::<crate::library::SongActivity>(&i.value) {
                    lib.song_activity.insert(i.key.clone(), activity);
                }
            }
            PREFERENCE => {
                let value = if gone { None } else { i.value.as_ref() };
                macro_rules! field { ($field:ident) => { lib.$field = value.and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default() }; }
                match i.key.as_str() {
                    "pinned" => field!(pinned),
                    "sidebar_hidden" => field!(sidebar_hidden),
                    "lyrics_offsets" => field!(lyrics_offsets),
                    "recent_lists" => field!(recent_lists),
                    "recent_artists" => field!(recent_artists),
                    "list_activity" => field!(list_activity),
                    "artist_plays" => field!(artist_plays),
                    "artist_skips" => field!(artist_skips),
                    "guest_imports" => {
                        if let Some(imports) = value.and_then(|v| serde_json::from_value::<BTreeSet<String>>(v.clone()).ok()) { lib.guest_imports.extend(imports); }
                    }
                    _ => continue,
                }
            }
            kind::LIKED => {
                lib.invalidate_liked_index();
                let pos = lib.liked.iter().position(|t| track_id(t) == i.key);
                match (gone, pos, from::<Track>(&i.value)) {
                    (true, Some(p), _) => {
                        lib.liked.remove(p);
                    }
                    (false, Some(p), Some(t)) => lib.liked[p] = t,
                    (false, None, Some(t)) => lib.liked.insert(0, t),
                    _ => continue,
                }
            }
            kind::PLAYLIST => {
                if lib.playlists.iter().any(|p| p.id == i.key && p.local_only) { continue; }
                let pos = lib.playlists.iter().position(|p| p.id == i.key);
                if gone {
                    state.known_playlists.remove(&i.key);
                    let Some(p) = pos else { continue };
                    let archived = lib.playlists.remove(p);
                    lib.playlist_trash.entry(i.key.clone()).or_insert(archived);
                } else {
                    let Some(v) = i.value.as_ref() else { continue };
                    let cover = v["cover"].as_str().and_then(synced_cover).map(|p| p.display().to_string());
                    let old_cover = pos.and_then(|p| lib.playlists[p].cover.clone());
                    let remote_tracks: Vec<Track> = serde_json::from_value(v["tracks"].clone()).unwrap_or_default();
                    let remote_ids: Vec<String> = remote_tracks.iter().map(track_id).collect();
                    let remote_base: Option<Vec<String>> = v.get("base").and_then(|b| serde_json::from_value(b.clone()).ok());
                    if remote_tracks.len() > crate::library::playlist_track_limit()
                        || pos.is_some_and(|p| lib.playlists[p].tracks.len() > crate::library::playlist_track_limit()) {
                        continue;
                    }
                    let mut tracks = match pos {
                        Some(p) => merge_playlist_tracks(
                            state.known_playlists.get(&i.key).map(Vec::as_slice),
                            remote_base.as_deref(),
                            &lib.playlists[p].tracks,
                            remote_tracks,
                        ),
                        None => remote_tracks,
                    };
                    let remote_membership: BTreeMap<String, crate::library::PlaylistMembership> = v.get("membership").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
                    let membership = lib.playlist_membership.entry(i.key.clone()).or_default();
                    for (id, op) in &remote_membership {
                        if membership.get(id).is_none_or(|old| op.newer_than(old)) { membership.insert(id.clone(), op.clone()); }
                    }
                    tracks.retain(|t| membership.get(&track_id(t)).is_none_or(|op| op.present));
                    tracks.truncate(crate::library::playlist_track_limit());
                    let merged_ids: Vec<String> = tracks.iter().map(track_id).collect();
                    if merged_ids != remote_ids || *membership != remote_membership {
                        unpushed_playlists.insert((i.kind.clone(), i.key.clone()), (hash(v), remote_ids));
                    } else {
                        state.known_playlists.insert(i.key.clone(), remote_ids);
                    }
                    let pl = Playlist {
                        local_only: false,
                        id: i.key.clone(),
                        name: v["name"].as_str().unwrap_or("Playlist").to_string(),
                        tracks,
                        origin: v["origin"].as_str().map(str::to_string),
                        // A picture not fetched yet keeps the old one.
                        cover: if v.get("cover").is_some() { cover.or(old_cover) } else { None },
                        // Device setting, never part of the synced value.
                        auto_download: pos.is_some_and(|p| lib.playlists[p].auto_download),
                        shared_by: v["shared_by"].as_str().map(str::to_string),
                    };
                    match pos {
                        Some(p) => lib.playlists[p] = pl,
                        None => lib.playlists.push(pl),
                    }
                }
            }
            kind::FOLLOWED => {
                let pos = lib.followed.iter().position(|a| a.name.trim().to_lowercase() == i.key);
                match (gone, pos, from::<FollowedArtist>(&i.value)) {
                    (true, Some(p), _) => {
                        lib.followed.remove(p);
                    }
                    (false, Some(p), Some(a)) => lib.followed[p] = a,
                    (false, None, Some(a)) => lib.followed.push(a),
                    _ => continue,
                }
            }
            kind::HIDDEN => {
                let pos = lib.hidden_artists.iter().position(|a| a.trim().to_lowercase() == i.key);
                let name = i.value.as_ref().and_then(|v| v["name"].as_str()).map(str::to_string);
                match (gone, pos, name) {
                    (true, Some(p), _) => {
                        lib.hidden_artists.remove(p);
                    }
                    (false, Some(p), Some(n)) => lib.hidden_artists[p] = n,
                    (false, None, Some(n)) => lib.hidden_artists.push(n),
                    _ => continue,
                }
            }
            kind::ALBUM => {
                let pos = lib.saved_albums.iter().position(|l| matches!(l, ListKey::Album { title, artist, .. } if album_key(title, artist) == i.key));
                let album = i.value.as_ref().map(|v| ListKey::Album {
                    title: v["title"].as_str().unwrap_or_default().to_string(),
                    artist: v["artist"].as_str().unwrap_or_default().to_string(),
                    tracks: serde_json::from_value(v["tracks"].clone()).unwrap_or_default(),
                });
                match (gone, pos, album) {
                    (true, Some(p), _) => {
                        lib.saved_albums.remove(p);
                    }
                    (false, Some(p), Some(a)) => lib.saved_albums[p] = a,
                    (false, None, Some(a)) => lib.saved_albums.insert(0, a),
                    _ => continue,
                }
            }
            kind::SEARCH => {
                let pos = lib.search_items.iter().position(|s| search_key(s) == i.key);
                match (gone, pos, from::<SearchItem>(&i.value)) {
                    (true, Some(p), _) => {
                        lib.search_items.remove(p);
                    }
                    (false, Some(p), Some(s)) => lib.search_items[p] = s,
                    (false, None, Some(s)) => {
                        lib.search_items.insert(0, s);
                        lib.search_items.truncate(SEARCH_HISTORY_LIMIT);
                    }
                    _ => continue,
                }
            }
            kind::DOWNLOAD => {
                let local = downloaded.iter().find(|t| track_id(t) == i.key).cloned();
                if gone {
                    state.pending_downloads.remove(&i.key);
                    // Downloaded files belong to this installation, not a session.
                } else if let (None, Some(t)) = (local, from::<Track>(&i.value)) {
                    state.pending_downloads.insert(i.key.clone(), t.clone());
                    out.download.push(t);
                }
                touched.push((i.kind.clone(), i.key.clone()));
                continue;
            }
            kind::AUTO_DOWNLOAD => {
                let on = !gone;
                if i.key == "liked" {
                    lib.liked_auto_download = on;
                } else if let Some(p) = lib.playlists.iter_mut().find(|p| p.id == i.key && !p.local_only) {
                    p.auto_download = on;
                } else {
                    continue;
                }
            }
            crate::stats::SYNC_KIND => {
                let device = crate::server_client::install_id();
                // This device's own months: what is here is newer.
                if let Some(month) = i.key.strip_prefix(&format!("{device}:")) {
                    if !gone {
                        if let Some(stats) = from::<crate::stats::MonthStats>(&i.value) {
                            lib.stats.months.entry(month.to_string()).or_default().merge_max(&stats);
                            out.library_changed = true;
                        }
                    }
                    state.agree(&(i.kind.clone(), i.key.clone()), if gone { None } else { i.value.as_ref() });
                    continue;
                }
                // Other devices' months are kept, never pushed back (they are
                // not in our snapshot, so they count as agreed-absent).
                if !lib.stats.apply_remote(&device, &i.key, if gone { None } else { i.value.as_ref() }) {
                    touched.push((i.kind.clone(), i.key.clone()));
                    continue;
                }
            }
            // Kinds from newer app versions are left alone.
            _ => continue,
        }
        out.library_changed = true;
        touched.push((i.kind.clone(), i.key.clone()));
    }
    // What the library now holds for those keys is the new agreement, except
    // playlists where local and remote tracks were merged and still need a push.
    let after = snapshot(lib, &state.wanted_downloads(downloaded));
    let map = after.map();
    for key in touched {
        if let Some((remote_hash, remote_ids)) = unpushed_playlists.remove(&key) {
            state.known.insert(joined(&key), remote_hash);
            state.known_playlists.insert(key.1, remote_ids);
        } else {
            let h = map.get(&key).map(|v| hash(v)).unwrap_or_else(|| DELETED.into());
            state.known.insert(joined(&key), h);
        }
    }
    out
}

/// Downloaded songs that reached the device leave the waiting list.
pub fn merge_guest(target: &mut Library, guest: &Library) {
    let fingerprint = sha_hex(&serde_json::to_vec(guest).unwrap_or_default());
    if target.guest_imports.contains(&fingerprint) { return; }
    let previous = target.clone();
    let snap = snapshot(guest, &[]);
    let items: Vec<_> = SyncState::default().changes(&snap, 0);
    apply(target, &mut SyncState::default(), &items, &[]);
    // A merge keeps both sets of personal list choices.
    for item in previous.pinned { if !target.pinned.contains(&item) { target.pinned.push(item); } }
    for item in previous.sidebar_hidden { if !target.sidebar_hidden.contains(&item) { target.sidebar_hidden.push(item); } }
    for item in previous.recent_lists { if !target.recent_lists.contains(&item) { target.recent_lists.push(item); } }
    target.recent_lists.truncate(crate::library::RECENT_LISTS_LIMIT);
    for (key, value) in previous.lyrics_offsets { target.lyrics_offsets.entry(key).or_insert(value); }
    for (key, value) in previous.artist_plays { let e = target.artist_plays.entry(key).or_default(); *e = (*e).max(value); }
    for (key, value) in previous.artist_skips { let e = target.artist_skips.entry(key).or_default(); *e = (*e).max(value); }
    for (key, value) in previous.list_activity { target.list_activity.entry(key).or_insert(value); }
    for artist in previous.recent_artists {
        if !target.recent_artists.iter().any(|a| a.name.eq_ignore_ascii_case(&artist.name)) { target.recent_artists.push(artist); }
    }
    for (key, month) in &guest.stats.months {
        let import_key = format!("{}:{key}", crate::server_client::install_id());
        let baseline = previous.guest_stats_imported.get(&import_key).cloned().unwrap_or_default();
        let mut combined = previous.stats.months.get(key).cloned().unwrap_or_default();
        combined.merge_guest(&month.since_guest_import(&baseline));
        target.stats.months.insert(key.clone(), combined);
        target.guest_stats_imported.entry(import_key).or_default().merge_max(month);
    }
    target.guest_imports.extend(previous.guest_imports);
    target.guest_imports.insert(fingerprint);
}

/// Downloaded songs that reached the device leave the waiting list.
pub fn settle_downloads(state: &mut SyncState, downloaded: &[Track]) {
    let have: BTreeSet<String> = downloaded.iter().map(track_id).collect();
    state.pending_downloads.retain(|id, _| !have.contains(id));
}

/// Split pushes so one request stays well under the server's body limit.
pub fn chunks(items: Vec<SyncItem>, max_items: usize, max_bytes: usize) -> Vec<Vec<SyncItem>> {
    let mut out: Vec<Vec<SyncItem>> = Vec::new();
    let mut cur: Vec<SyncItem> = Vec::new();
    let mut size = 0;
    for i in items {
        let n = serde_json::to_vec(&i).map(|b| b.len()).unwrap_or(0);
        if !cur.is_empty() && (cur.len() >= max_items || size + n > max_bytes) {
            out.push(std::mem::take(&mut cur));
            size = 0;
        }
        size += n;
        cur.push(i);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbox_retry_preserves_operation_time_across_serialization() {
        let lib = lib_with(&["Offline"]);
        let snap = snapshot(&lib, &[]);
        let mut state = SyncState::default();
        let first = state.enqueue(&snap, 100);
        let mut restarted: SyncState = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        let retry = restarted.enqueue(&snap, 999);
        assert_eq!(retry[0].updated_at, first[0].updated_at);
        restarted.mark_pushed(&retry);
        assert!(restarted.outbox.is_empty());
        assert!(restarted.enqueue(&snap, 1000).is_empty());
    }

    #[test]
    fn playlist_removal_survives_stale_edits_and_can_be_explicitly_readded() {
        let mut local = Library::default();
        let song = test_track("Removed", "Artist");
        let id = local.create_playlist("Shared").id.clone();
        local.add_to_playlist(&id, song.clone());
        local.checkpoint_playlists();
        let mut stale = local.clone();
        let mut state = SyncState::default();
        state.mark_pushed(&state.changes(&snapshot(&local, &[]), 10));
        local.remove_from_playlist(&id, 0);
        local.checkpoint_playlists();
        let items = state.changes(&snapshot(&local, &[]), 20);
        apply(&mut stale, &mut SyncState::default(), &items, &[]);
        assert!(stale.playlist(&id).unwrap().tracks.is_empty());
        let mut old_library = Library::default();
        old_library.playlists.push(Playlist { tracks: vec![song.clone()], ..local.playlist(&id).unwrap().clone() });
        let old = SyncState::default().changes(&snapshot(&old_library, &[]), 30);
        apply(&mut stale, &mut SyncState::default(), &old, &[]);
        assert!(stale.playlist(&id).unwrap().tracks.is_empty());
        stale.add_to_playlist(&id, song);
        stale.checkpoint_playlists();
        assert!(stale.playlist_membership[&id].values().all(|op| op.present));
    }

    #[test]
    fn trash_versions_and_explicit_feedback_sync_and_restore() {
        let mut source = Library::default();
        let id = source.create_playlist("Keep me").id.clone();
        source.add_to_playlist(&id, test_track("One", "Artist"));
        source.checkpoint_playlists();
        source.recommendation_feedback.insert("artist".into(), -2);
        source.delete_playlist(&id);
        let items = SyncState::default().changes(&snapshot(&source, &[]), 1);
        let mut target = Library::default();
        apply(&mut target, &mut SyncState::default(), &items, &[]);
        assert!(crate::taste::Taste::from_library(&target).is_hidden("Artist"));
        assert!(!target.playlist_versions.is_empty());
        assert!(target.restore_playlist(&id));
        assert_eq!(target.playlist(&id).unwrap().tracks.len(), 1);
    }

    #[test]
    fn listening_and_recommendation_actions_survive_account_sync() {
        let mut source = Library::default();
        let song = test_track("Remember", "Artist");
        source.push_recent(song.clone());
        source.record_play(&song);
        source.record_listen(&song, 1000);
        source.record_skip(&song);
        source.pinned.push(ListKey::Liked);
        source.sidebar_hidden.push("recent".into());
        source.lyrics_offsets.insert("remember|artist".into(), 80);
        let snap = snapshot(&source, &[]);
        let items = SyncState::default().changes(&snap, 1000);
        let mut target = Library::default();
        let mut state = SyncState::default();
        assert!(apply(&mut target, &mut state, &items, &[]).library_changed);
        assert_eq!(target.recent, source.recent);
        assert_eq!(target.song_activity, source.song_activity);
        assert_eq!(target.artist_plays, source.artist_plays);
        assert_eq!(target.artist_skips, source.artist_skips);
        assert_eq!(target.pinned, source.pinned);
        assert_eq!(target.sidebar_hidden, source.sidebar_hidden);
        assert_eq!(target.lyrics_offsets, source.lyrics_offsets);
        assert!(state.changes(&snapshot(&target, &[]), 2000).is_empty());
    }

    #[test]
    fn merging_guest_preserves_account_choices_and_does_not_double_count() {
        let _env = crate::paths::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut guest = Library::default();
        let song = test_track("Guest", "Artist");
        guest.toggle_like(&song);
        guest.pinned.push(ListKey::Recent);
        guest.stats.months.entry("2026-10".into()).or_default().ms = 1000;
        let mut account = Library::default();
        account.pinned.push(ListKey::Liked);
        account.stats.months.entry("2026-10".into()).or_default().ms = 2000;
        merge_guest(&mut account, &guest);
        assert!(account.is_liked(&song));
        assert!(account.pinned.contains(&ListKey::Liked) && account.pinned.contains(&ListKey::Recent));
        assert_eq!(account.stats.months["2026-10"].ms, 3000);
        merge_guest(&mut account, &guest);
        assert_eq!(account.stats.months["2026-10"].ms, 3000);
        assert_eq!(guest.stats.months["2026-10"].ms, 1000);
        guest.stats.months.get_mut("2026-10").unwrap().ms = 1500;
        merge_guest(&mut account, &guest);
        assert_eq!(account.stats.months["2026-10"].ms, 3500);
    }

    #[test]
    fn local_playlist_and_download_files_are_not_removed_by_account_tombstones() {
        let mut lib = Library::default();
        let song = test_track("Offline", "Artist");
        let playlist = lib.create_playlist("Local");
        playlist.local_only = true;
        playlist.tracks.push(song.clone());
        let id = playlist.id.clone();
        let snap = snapshot(&lib, &[]);
        assert!(!snap.items.iter().any(|(key, _)| key.0 == kind::PLAYLIST));
        let tombstone = |kind: &str, key: String| SyncItem { kind: kind.into(), key, value: None, deleted: true, updated_at: 10, rev: 1 };
        let result = apply(&mut lib, &mut SyncState::default(), &[tombstone(kind::PLAYLIST, id.clone()), tombstone(kind::DOWNLOAD, track_id(&song))], &[song]);
        assert!(lib.playlist(&id).unwrap().local_only);
        assert!(result.remove_downloads.is_empty());
    }

    #[test]
    fn synced_metadata_invalidates_liked_lookup_without_length_change() {
        let mut lib = lib_with(&["One", "Two", "Three", "Four", "Five"]);
        let mut remote = lib.liked[0].clone();
        let mut alias = remote.clone();
        alias.source = ProviderId::YouTube;
        alias.id = "new-youtube-id".into();
        alias.title = "Different metadata".into();
        assert!(!lib.is_liked(&alias));
        remote.external_ids.insert(alias.source, alias.id.clone());
        let item = SyncItem { kind: kind::LIKED.into(), key: track_id(&remote),
            value: Some(serde_json::to_value(remote).unwrap()), updated_at: 1, deleted: false, rev: 1 };
        apply(&mut lib, &mut SyncState::default(), &[item], &[]);
        assert_eq!(lib.liked.len(), 5);
        assert!(lib.is_liked(&alias));
    }

    fn test_track(title: &str, artist: &str) -> Track {
        crate::dedupe::test_track(ProviderId::Audius, &title.to_lowercase(), title, artist, Some(200))
    }

    fn lib_with(tracks: &[&str]) -> Library {
        let mut l = Library::default();
        for t in tracks.iter().rev() {
            l.toggle_like(&test_track(t, "Artist"));
        }
        l
    }

    /// Simulates a server: applies LWW and hands out revisions.
    #[derive(Default)]
    struct Server {
        items: BTreeMap<Key, SyncItem>,
        rev: u64,
    }
    impl Server {
        fn push(&mut self, items: &[SyncItem]) -> Vec<SyncItem> {
            let mut ok = vec![];
            for i in items {
                let key = (i.kind.clone(), i.key.clone());
                if self.items.get(&key).is_none_or(|o| i.updated_at > o.updated_at) {
                    self.rev += 1;
                    self.items.insert(key, SyncItem { rev: self.rev, ..i.clone() });
                    ok.push(i.clone());
                }
            }
            ok
        }
        fn pull(&self, since: u64) -> Vec<SyncItem> {
            let mut v: Vec<SyncItem> = self.items.values().filter(|i| i.rev > since).cloned().collect();
            v.sort_by_key(|i| i.rev);
            v
        }
    }

    fn round(srv: &mut Server, lib: &mut Library, st: &mut SyncState, now: u64) -> Applied {
        let snap = snapshot(lib, &[]);
        let ch = st.changes(&snap, now);
        let ok = srv.push(&ch);
        st.mark_pushed(&ok);
        let items = srv.pull(st.last_rev);
        st.last_rev = srv.rev;
        apply(lib, st, &items, &[])
    }

    #[test]
    fn two_devices_converge_and_keep_order() {
        let mut srv = Server::default();
        let (mut a, mut sa) = (lib_with(&["One", "Two", "Three"]), SyncState::default());
        a.create_playlist("Mix").tracks.push(test_track("Four", "Artist"));
        a.toggle_follow("Daft Punk", None);
        let (mut b, mut sb) = (Library::default(), SyncState::default());
        round(&mut srv, &mut a, &mut sa, 10);
        let applied = round(&mut srv, &mut b, &mut sb, 11);
        assert!(applied.library_changed);
        let titles: Vec<&str> = b.liked.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, vec!["One", "Two", "Three"]);
        assert_eq!(b.playlists.len(), 1);
        assert_eq!(b.playlists[0].tracks.len(), 1);
        assert!(b.is_following("daft punk"));
        // Nothing left to push on either side: no echo.
        assert!(sb.changes(&snapshot(&b, &[]), 12).is_empty());
        assert!(sa.changes(&snapshot(&a, &[]), 12).is_empty());

        // B unlikes "Two" and hides an artist; A picks it up.
        let two = b.liked[1].clone();
        b.toggle_like(&two);
        b.toggle_hidden("Nickelback");
        round(&mut srv, &mut b, &mut sb, 20);
        round(&mut srv, &mut a, &mut sa, 21);
        assert!(!a.is_liked(&two));
        assert!(a.is_hidden("nickelback"));
        assert!(sa.changes(&snapshot(&a, &[]), 22).is_empty());
    }

    #[test]
    fn first_sign_in_merges_instead_of_deleting() {
        let mut srv = Server::default();
        let (mut a, mut sa) = (lib_with(&["A1"]), SyncState::default());
        round(&mut srv, &mut a, &mut sa, 10);
        let (mut b, mut sb) = (lib_with(&["B1"]), SyncState::default());
        round(&mut srv, &mut b, &mut sb, 11);
        round(&mut srv, &mut a, &mut sa, 12);
        assert_eq!(a.liked.len(), 2);
        assert_eq!(b.liked.len(), 2);
    }

    #[test]
    fn downloads_are_requested_but_files_remain_local() {
        let mut srv = Server::default();
        let song = test_track("Offline", "Artist");
        let (mut a, mut sa) = (Library::default(), SyncState::default());
        let snap = snapshot(&a, &[song.clone()]);
        let ok = srv.push(&sa.changes(&snap, 10));
        sa.mark_pushed(&ok);
        let (mut b, mut sb) = (Library::default(), SyncState::default());
        let applied = apply(&mut b, &mut sb, &srv.pull(0), &[]);
        assert_eq!(applied.download, vec![song.clone()]);
        assert!(!applied.library_changed);
        // While it downloads, B must not tombstone it.
        let wanted = sb.wanted_downloads(&[]);
        assert!(sb.changes(&snapshot(&b, &wanted), 11).is_empty());
        settle_downloads(&mut sb, &[song.clone()]);
        assert!(sb.pending_downloads.is_empty());
        assert!(sb.changes(&snapshot(&b, &[song.clone()]), 11).is_empty());
        // A removes its download; B keeps its own downloaded file.
        let ch = sa.changes(&snapshot(&a, &[]), 20);
        assert!(ch[0].deleted);
        srv.push(&ch);
        let applied = apply(&mut b, &mut sb, &srv.pull(1), &[song.clone()]);
        assert!(applied.remove_downloads.is_empty());
        let _ = &mut a;
    }

    #[test]
    fn unknown_kinds_are_ignored_and_state_is_per_account() {
        let mut lib = Library::default();
        let mut st = SyncState::default();
        let item = SyncItem { kind: "future_thing".into(), key: "x".into(), value: Some(serde_json::json!(1)), updated_at: 1, deleted: false, rev: 1 };
        assert_eq!(apply(&mut lib, &mut st, &[item], &[]), Applied::default());

        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.json");
        let mut s = SyncState { account_id: "u1".into(), last_rev: 9, ..Default::default() };
        s.known.insert("liked\u{1f}a".into(), "h".into());
        std::fs::write(&p, serde_json::to_vec(&s).unwrap()).unwrap();
        assert_eq!(SyncState::load_from(&p, "u1").last_rev, 9);
        assert_eq!(SyncState::load_from(&p, "u2").last_rev, 0);
    }

    #[test]
    fn chunking() {
        let it = |n: usize| SyncItem { kind: "liked".into(), key: n.to_string(), value: Some(serde_json::json!("x".repeat(100))), updated_at: 1, deleted: false, rev: 0 };
        let c = chunks((0..10).map(it).collect(), 4, usize::MAX);
        assert_eq!(c.iter().map(Vec::len).collect::<Vec<_>>(), vec![4, 4, 2]);
        let c = chunks((0..10).map(it).collect(), 100, 300);
        assert!(c.len() >= 4 && c.iter().all(|v| !v.is_empty()));
    }

    #[test]
    fn concurrent_playlist_edits_merge_without_losing_tracks() {
        let mut srv = Server::default();
        let (mut a, mut sa) = (Library::default(), SyncState::default());
        let (mut b, mut sb) = (Library::default(), SyncState::default());

        a.create_playlist("Shared").tracks.push(test_track("Base", "Artist"));
        round(&mut srv, &mut a, &mut sa, 10);
        round(&mut srv, &mut b, &mut sb, 11);

        // Concurrently, A adds Track X and B adds Track Y to the same playlist.
        a.playlists[0].tracks.push(test_track("TrackX", "Artist"));
        b.playlists[0].tracks.push(test_track("TrackY", "Artist"));

        round(&mut srv, &mut a, &mut sa, 20);
        round(&mut srv, &mut b, &mut sb, 21);
        // A pulls B's update and merges TrackX + TrackY, then pushes the merged list.
        round(&mut srv, &mut a, &mut sa, 22);
        round(&mut srv, &mut a, &mut sa, 23);
        round(&mut srv, &mut b, &mut sb, 24);

        let a_titles: Vec<&str> = a.playlists[0].tracks.iter().map(|t| t.title.as_str()).collect();
        let b_titles: Vec<&str> = b.playlists[0].tracks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(a_titles, vec!["Base", "TrackY", "TrackX"]);
        assert_eq!(b_titles, vec!["Base", "TrackY", "TrackX"]);
        assert!(sa.changes(&snapshot(&a, &[]), 25).is_empty());
        assert!(sb.changes(&snapshot(&b, &[]), 25).is_empty());
    }
}
