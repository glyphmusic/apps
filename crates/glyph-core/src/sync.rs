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

fn provider_str(p: ProviderId) -> String {
    serde_json::to_value(p).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// Stable id of a song across devices.
pub fn track_id(t: &Track) -> String {
    format!("{}:{}", provider_str(t.source), t.id)
}

fn hash(v: &Value) -> String {
    let d = Sha256::digest(v.to_string().as_bytes());
    d[..12].iter().map(|b| format!("{b:02x}")).collect()
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
    for t in lib.liked.iter().rev() {
        push(k(kind::LIKED, track_id(t)), serde_json::to_value(t).unwrap_or_default());
    }
    for p in &lib.playlists {
        let mut v = serde_json::json!({ "name": p.name, "tracks": p.tracks, "origin": p.origin, "shared_by": p.shared_by });
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
    for p in lib.playlists.iter().filter(|p| p.auto_download) {
        push(k(kind::AUTO_DOWNLOAD, p.id.clone()), serde_json::json!({ "on": true }));
    }
    s
}

/// What this device last agreed on with the account, persisted next to the library.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncState {
    pub account_id: String,
    pub last_rev: u64,
    /// `kind \u{1f} key` -> value hash (`-` = deleted).
    #[serde(default)]
    pub known: BTreeMap<String, String>,
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
        paths::data_dir().join("sync_state.json")
    }

    /// State for `account_id`; a different account (or none) starts fresh.
    pub fn load(account_id: &str) -> Self {
        Self::load_from(&Self::path(), account_id)
    }

    pub fn load_from(path: &Path, account_id: &str) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<SyncState>(&b).ok())
            .filter(|s| s.account_id == account_id)
            .unwrap_or_else(|| SyncState { account_id: account_id.into(), ..Default::default() })
    }

    pub fn save(&self) -> anyhow::Result<()> {
        paths::write_json(&Self::path(), self)
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
                out.push(SyncItem { kind: key.0.clone(), key: key.1.clone(), value: Some(v.clone()), updated_at: now, deleted: false, rev: 0 });
            }
        }
        for (j, h) in &self.known {
            let Some(key) = split(j) else { continue };
            if h != DELETED && !map.contains_key(&key) {
                out.push(SyncItem { kind: key.0, key: key.1, value: None, updated_at: now, deleted: true, rev: 0 });
            }
        }
        out
    }

    /// The server accepted these pushed items.
    pub fn mark_pushed(&mut self, items: &[SyncItem]) {
        for i in items {
            let h = match (&i.value, i.deleted) {
                (Some(v), false) => hash(v),
                _ => DELETED.into(),
            };
            self.known.insert(joined(&(i.kind.clone(), i.key.clone())), h);
        }
    }

    /// Record `value` (None = deleted) as agreed with the server for `key`.
    pub fn agree(&mut self, key: &Key, value: Option<&Value>) {
        self.known.insert(joined(key), value.map(hash).unwrap_or_else(|| DELETED.into()));
    }

    /// Forget the agreement for `key`, so a missing local item is not pushed
    /// as a deletion.
    pub fn forget(&mut self, key: &Key) {
        self.known.remove(&joined(key));
    }

    /// Start over (server reset or a different account): keep nothing agreed.
    pub fn reset(&mut self) {
        self.last_rev = 0;
        self.known.clear();
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
    // Auto-download switches go last: their playlist may arrive in the
    // same batch.
    let (switches, items): (Vec<&SyncItem>, Vec<&SyncItem>) = items.iter().partition(|i| i.kind == kind::AUTO_DOWNLOAD);
    for i in items.into_iter().chain(switches) {
        let gone = i.deleted || i.value.is_none();
        match i.kind.as_str() {
            kind::LIKED => {
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
                let pos = lib.playlists.iter().position(|p| p.id == i.key);
                if gone {
                    let Some(p) = pos else { continue };
                    lib.playlists.remove(p);
                } else {
                    let Some(v) = i.value.as_ref() else { continue };
                    let cover = v["cover"].as_str().and_then(synced_cover).map(|p| p.display().to_string());
                    let old_cover = pos.and_then(|p| lib.playlists[p].cover.clone());
                    let pl = Playlist {
                        id: i.key.clone(),
                        name: v["name"].as_str().unwrap_or("Playlist").to_string(),
                        tracks: serde_json::from_value(v["tracks"].clone()).unwrap_or_default(),
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
                    out.remove_downloads.extend(local);
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
                } else if let Some(p) = lib.playlists.iter_mut().find(|p| p.id == i.key) {
                    p.auto_download = on;
                } else {
                    continue;
                }
            }
            // Kinds from newer app versions are left alone.
            _ => continue,
        }
        out.library_changed = true;
        touched.push((i.kind.clone(), i.key.clone()));
    }
    // What the library now holds for those keys is the new agreement.
    let after = snapshot(lib, &state.wanted_downloads(downloaded));
    let map = after.map();
    for key in touched {
        let h = map.get(&key).map(|v| hash(v)).unwrap_or_else(|| DELETED.into());
        state.known.insert(joined(&key), h);
    }
    out
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
    fn downloads_are_requested_and_removed() {
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
        // A removes the download -> B deletes its file.
        let ch = sa.changes(&snapshot(&a, &[]), 20);
        assert!(ch[0].deleted);
        srv.push(&ch);
        let applied = apply(&mut b, &mut sb, &srv.pull(1), &[song.clone()]);
        assert_eq!(applied.remove_downloads, vec![song]);
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
}
