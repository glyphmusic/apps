//! Local library: liked songs, recently played (last 50), playlists, per-song
//! lyrics offsets. Persisted as JSON in the app data dir. Never leaves the device.

use crate::dedupe::{same_song, track_key, TrackIndex};
use crate::paths;

/// Applies to every user playlist, including imports and re-imports.
pub const MAX_PLAYLIST_TRACKS: usize = 5_000;
static PLAYLIST_TRACK_LIMIT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(MAX_PLAYLIST_TRACKS);
pub fn playlist_track_limit() -> usize {
    PLAYLIST_TRACK_LIMIT.load(std::sync::atomic::Ordering::Relaxed)
}
pub(crate) fn set_playlist_track_limit(limit: usize) {
    PLAYLIST_TRACK_LIMIT.store(limit.clamp(1, 50_000), std::sync::atomic::Ordering::Relaxed);
}
use glyph_protocol::Track;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Default)]
struct LikedCache(Mutex<Option<(usize, TrackIndex)>>);

impl Clone for LikedCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl PartialEq for LikedCache {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// Max entries in "recently played".
pub const RECENT_LIMIT: usize = 50;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Playlist {
    /// Available on this installation across accounts; never uploaded by sync.
    #[serde(default)]
    pub local_only: bool,
    pub id: String,
    pub name: String,
    pub tracks: Vec<Track>,
    /// Where it was imported from (URL), so a re-import only adds new songs.
    pub origin: Option<String>,
    /// Picture chosen by the user (local file in the data dir).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
    /// Songs in it are downloaded automatically, newly added ones included.
    /// Synced (kind `autodl`) for signed-in users.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_download: bool,
    /// Handle of whoever shared it (opened from a short link); shown as the author.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaylistRevision {
    pub playlist: Playlist,
    pub saved_at: u64,
}

/// Per-song membership operation. Timestamp + installation id give every
/// operation a stable order; a removal wins an otherwise identical tie.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistMembership {
    pub at: u64,
    pub device: String,
    pub present: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct LocalPlaylistArchive {
    trash: BTreeMap<String, Playlist>,
    versions: BTreeMap<String, PlaylistRevision>,
    membership: BTreeMap<String, BTreeMap<String, PlaylistMembership>>,
}

impl PlaylistMembership {
    pub fn newer_than(&self, other: &Self) -> bool {
        (self.at, &self.device, !self.present) > (other.at, &other.device, !other.present)
    }
}

/// A song list the library can point at (sidebar "recent", saved albums).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ListKey {
    Liked,
    Recent,
    Playlist { id: String },
    Album { title: String, artist: String, tracks: Vec<Track> },
    ArtistLiked { artist: String },
    /// Songs saved for offline use.
    Downloads,
}

impl ListKey {
    /// Same list (albums compare by title + artist, not by their songs).
    pub fn same(&self, other: &ListKey) -> bool {
        match (self, other) {
            (ListKey::Album { title: a, artist: b, .. }, ListKey::Album { title: c, artist: d, .. }) => {
                a.eq_ignore_ascii_case(c) && b.eq_ignore_ascii_case(d)
            }
            _ => self == other,
        }
    }
}

/// One entry of "recent searches": a song or an artist picked from search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SearchItem {
    Song { track: Track },
    Artist { name: String, #[serde(default, skip_serializing_if = "Option::is_none")] track: Option<Track> },
}

impl SearchItem {
    fn same(&self, other: &SearchItem) -> bool {
        match (self, other) {
            (SearchItem::Song { track: a }, SearchItem::Song { track: b }) => same_entry(a, b),
            (SearchItem::Artist { name: a, .. }, SearchItem::Artist { name: b, .. }) => a.eq_ignore_ascii_case(b),
            _ => false,
        }
    }
}

/// One entry of "recently played": a song, or an artist whose page started
/// playback (so Home can show the artist next to the songs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecentItem {
    Song { track: Track },
    Artist {
        name: String,
        /// A song of theirs, for the picture until the artist page has a photo.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        track: Option<Track>,
    },
}

impl RecentItem {
    /// The song behind this entry (an artist's first song as its picture).
    pub fn track(&self) -> Option<&Track> {
        match self {
            RecentItem::Song { track } => Some(track),
            RecentItem::Artist { track, .. } => track.as_ref(),
        }
    }

    pub fn artist_name(&self) -> Option<&str> {
        match self {
            RecentItem::Artist { name, .. } => Some(name),
            RecentItem::Song { .. } => None,
        }
    }

    fn same(&self, other: &RecentItem) -> bool {
        match (self, other) {
            (RecentItem::Song { track: a }, RecentItem::Song { track: b }) => same_entry(a, b),
            (RecentItem::Artist { name: a, .. }, RecentItem::Artist { name: b, .. }) => a.eq_ignore_ascii_case(b.trim()),
            _ => false,
        }
    }
}

/// "Recently played" before it held artists was a plain list of songs; keep
/// reading those files (an entry with a `kind` is the new tagged form).
#[derive(Deserialize)]
#[serde(untagged)]
enum RecentItemCompat {
    Item(RecentItem),
    Song(Track),
}

fn deserialize_recent<'de, D>(deserializer: D) -> Result<Vec<RecentItem>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Vec::<RecentItemCompat>::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .map(|r| match r {
            RecentItemCompat::Item(i) => i,
            RecentItemCompat::Song(track) => RecentItem::Song { track },
        })
        .collect())
}

/// An artist the user follows (shown in the library and on Home).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FollowedArtist {
    pub name: String,
    /// A song of theirs, for the picture until the artist page has a photo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<Track>,
}

/// Max entries in "recently opened lists".
pub const RECENT_LISTS_LIMIT: usize = 24;

/// Local listening activity; older listens lose half their weight every week.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SongActivity {
    pub track: Track,
    pub score: f64,
    pub last_listened: u64,
}
impl SongActivity {
    fn weight(&self, now: u64) -> f64 {
        if !self.score.is_finite() || self.score < 0.0 { return 0.0; }
        self.score * (-((now.saturating_sub(self.last_listened)) as f64) / (7.0 * 86400.0)).exp2()
    }
}

#[derive(Debug, Clone, Default)]
pub struct LibraryPath(pub Option<PathBuf>);
impl PartialEq for LibraryPath {
    fn eq(&self, _: &Self) -> bool { true }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Library {
    #[serde(default)]
    pub playlist_membership: BTreeMap<String, BTreeMap<String, PlaylistMembership>>,
    /// Immutable playlist checkpoints, keyed by content hash.
    #[serde(default)]
    pub playlist_versions: BTreeMap<String, PlaylistRevision>,
    #[serde(default)]
    pub playlist_trash: BTreeMap<String, Playlist>,
    /// Explicit recommendation preference by normalized artist: -2 block,
    /// -1 less, +1 more. Removing an entry resets the preference.
    #[serde(default)]
    pub recommendation_feedback: BTreeMap<String, i8>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub guest_imports: std::collections::BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub guest_stats_imported: BTreeMap<String, crate::stats::MonthStats>,
    /// Captured by every asynchronous save, so account switches cannot redirect it.
    #[serde(skip)]
    pub storage_path: LibraryPath,
    pub liked: Vec<Track>,
    /// Per-device activity; excluded from account sync. Capped at 500 songs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub song_activity: BTreeMap<String, SongActivity>,
    #[serde(default, deserialize_with = "deserialize_recent")]
    pub recent: Vec<RecentItem>,
    pub playlists: Vec<Playlist>,
    /// Lyrics offsets in ms, keyed by normalised `title|artist`.
    #[serde(default)]
    pub lyrics_offsets: BTreeMap<String, i64>,
    /// Songs picked from search, newest first (the search bar's dropdown).
    #[serde(default)]
    pub search_history: Vec<Track>,
    /// Albums saved with the heart (shown in the library).
    #[serde(default)]
    pub saved_albums: Vec<ListKey>,
    /// Lists opened recently, newest first (sidebar).
    #[serde(default)]
    pub recent_lists: Vec<ListKey>,
    /// Lists pinned to the top of the sidebar and the library.
    #[serde(default)]
    pub pinned: Vec<ListKey>,
    /// Liked Songs was downloaded once: newly liked songs download too.
    #[serde(default)]
    pub liked_auto_download: bool,
    /// How much each list is used (listening from it, opening it): list id
    /// -> (score, last use, Unix s). Orders the sidebar; fades over weeks.
    #[serde(default)]
    pub list_activity: BTreeMap<String, (f64, u64)>,
    /// Followed artists, newest first.
    #[serde(default)]
    pub followed: Vec<FollowedArtist>,
    /// Recent searches (songs and artists), newest first. Replaces
    /// `search_history`, which is migrated on load.
    #[serde(default)]
    pub search_items: Vec<SearchItem>,
    /// Artists the user chose not to see (search, radio).
    #[serde(default)]
    pub hidden_artists: Vec<String>,
    /// Songs started per artist (normalised name), for the taste profile.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub artist_plays: BTreeMap<String, u32>,
    /// Songs skipped within 30 s per artist (normalised name).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub artist_skips: BTreeMap<String, u32>,
    /// Artist pages opened lately, newest first (sidebar next to the lists).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_artists: Vec<FollowedArtist>,
    /// Sidebar entries the user hid ([`Library::list_id`] / [`Library::artist_id`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sidebar_hidden: Vec<String>,
    /// Opt-in listening statistics (empty while turned off).
    #[serde(default, skip_serializing_if = "crate::stats::Stats::is_empty")]
    pub stats: crate::stats::Stats,
    /// Transient lookup index for `liked` so `is_liked` is O(1) during UI rendering.
    #[serde(skip)]
    liked_cache: LikedCache,
}

/// Lightweight local activity snapshot persisted separately on high-frequency
/// playback and navigation events so the full library (`liked`, `playlists`,
/// `saved_albums`, etc.) is not cloned and pretty-printed on every listen.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LibraryActivity {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub song_activity: BTreeMap<String, SongActivity>,
    #[serde(default, deserialize_with = "deserialize_recent")]
    pub recent: Vec<RecentItem>,
    #[serde(default)]
    pub recent_lists: Vec<ListKey>,
    #[serde(default)]
    pub list_activity: BTreeMap<String, (f64, u64)>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub artist_plays: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub artist_skips: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_artists: Vec<FollowedArtist>,
    #[serde(default, skip_serializing_if = "crate::stats::Stats::is_empty")]
    pub stats: crate::stats::Stats,
}

impl LibraryActivity {
    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&Library::activity_path())
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec(self)?;
        paths::write_atomic(path, &bytes)
    }
}

/// Max entries in the search history.
pub const SEARCH_HISTORY_LIMIT: usize = 20;
/// Longest playlist name in characters (longer ones are cut).
pub const PLAYLIST_NAME_MAX: usize = 100;
/// Artist pages remembered for the sidebar.
const RECENT_ARTISTS_LIMIT: usize = 24;

/// A playlist name as stored: trimmed, at most [`PLAYLIST_NAME_MAX`] characters.
/// An album key without songs (nothing to play, not worth saving).
fn tracks_empty(key: &ListKey) -> bool {
    matches!(key, ListKey::Album { tracks, .. } if tracks.is_empty())
}

pub fn clean_playlist_name(name: &str) -> String {
    name.trim().chars().take(PLAYLIST_NAME_MAX).collect::<String>().trim_end().to_string()
}

impl Library {
    pub fn path() -> PathBuf {
        paths::data_dir().join("library.json")
    }

    pub fn guest_path() -> PathBuf {
        if paths::data_dir().join("legacy-library-account.json").exists() {
            paths::data_dir().join("guest").join("library.json")
        } else { Self::path() }
    }

    pub fn activity_path() -> PathBuf {
        Self::activity_path_for(&Self::path())
    }

    pub fn activity_path_for(path: &Path) -> PathBuf {
        path.with_extension("activity.json")
    }

    pub fn activity_snapshot(&self) -> LibraryActivity {
        LibraryActivity {
            song_activity: self.song_activity.clone(),
            recent: self.recent.clone(),
            recent_lists: self.recent_lists.clone(),
            list_activity: self.list_activity.clone(),
            artist_plays: self.artist_plays.clone(),
            artist_skips: self.artist_skips.clone(),
            recent_artists: self.recent_artists.clone(),
            stats: self.stats.clone(),
        }
    }

    pub fn apply_activity(&mut self, act: LibraryActivity) {
        self.song_activity = act.song_activity;
        self.recent = act.recent;
        self.recent_lists = act.recent_lists;
        self.list_activity = act.list_activity;
        self.artist_plays = act.artist_plays;
        self.artist_skips = act.artist_skips;
        if !act.recent_artists.is_empty() {
            self.recent_artists = act.recent_artists;
        }
        if !act.stats.is_empty() {
            self.stats = act.stats;
        }
    }

    /// Missing file = empty library. A corrupt file is kept as `library.json.bak`.
    pub fn load() -> anyhow::Result<Self> {
        let account_path = crate::account::library_path();
        if account_path.is_some() && crate::account::stored_guest_merge_pending() {
            let mut guest = Self::load_from(&Self::guest_path())?;
            guest.load_local_playlists()?;
            return Ok(guest);
        }
        let marker = paths::data_dir().join("legacy-library-account.json");
        if let Some(path) = account_path.as_ref().filter(|path| !path.exists() && Self::path().exists() && !marker.exists()) {
            // Preserve the old files as an archive, but never expose an old
            // signed-in library as the guest's data after the migration.
            let legacy = Self::load_from(&Self::path())?;
            legacy.save_to(path)?;
            paths::write_json(&marker, &path.display().to_string())?;
        }
        let mut lib = Self::load_from(&account_path.unwrap_or_else(Self::guest_path))?;
        lib.load_local_playlists()?;
        Ok(lib)
    }

    pub fn load_from(path: &Path) -> anyhow::Result<Self> {
        let mut lib = match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice::<Library>(&bytes) {
                Ok(mut lib) => {
                    // Older files only had songs in the search history.
                    if lib.search_items.is_empty() && !lib.search_history.is_empty() {
                        lib.search_items = lib.search_history.drain(..).map(|track| SearchItem::Song { track }).collect();
                    }
                    lib
                }
                Err(e) => {
                    let _ = std::fs::copy(path, path.with_extension("json.bak"));
                    return Err(anyhow::anyhow!("library file is corrupt (backup kept): {e}"));
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(e.into()),
        };
        if let Ok(act_bytes) = std::fs::read(Self::activity_path_for(path)) {
            if let Ok(act) = serde_json::from_slice::<LibraryActivity>(&act_bytes) {
                lib.apply_activity(act);
            }
        }
        lib.storage_path.0 = Some(path.to_path_buf());
        lib.checkpoint_playlists();
        Ok(lib)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&self.storage_path.0.clone().unwrap_or_else(Self::path))?;
        paths::write_json(&paths::data_dir().join("local-playlists.json"), &self.playlists.iter().filter(|p| p.local_only).collect::<Vec<_>>())?;
        let archive = LocalPlaylistArchive {
            trash: self.playlist_trash.iter().filter(|(_, p)| p.local_only).map(|(k,v)| (k.clone(),v.clone())).collect(),
            versions: self.playlist_versions.iter().filter(|(_, v)| v.playlist.local_only).map(|(k,v)| (k.clone(),v.clone())).collect(),
            membership: self.playlist_membership.iter().filter(|(id, _)| self.playlist(id).or_else(|| self.playlist_trash.get(*id)).is_some_and(|p| p.local_only)).map(|(k,v)| (k.clone(),v.clone())).collect(),
        };
        paths::write_json(&paths::data_dir().join("local-playlist-archive.json"), &archive)
    }

    pub fn load_local_playlists(&mut self) -> anyhow::Result<()> {
        let path = paths::data_dir().join("local-playlists.json");
        let local: Vec<Playlist> = match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        self.playlists.retain(|p| !p.local_only);
        self.playlists.extend(local.into_iter().filter(|p| p.local_only));
        match std::fs::read(paths::data_dir().join("local-playlist-archive.json")) {
            Ok(bytes) => {
                let archive: LocalPlaylistArchive = serde_json::from_slice(&bytes)?;
                self.playlist_trash.retain(|_, p| !p.local_only);
                self.playlist_versions.retain(|_, v| !v.playlist.local_only);
                self.playlist_trash.extend(archive.trash.into_iter().filter(|(_, p)| p.local_only));
                self.playlist_versions.extend(archive.versions.into_iter().filter(|(_, v)| v.playlist.local_only));
                self.playlist_membership.extend(archive.membership);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        self.checkpoint_playlists();
        Ok(())
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        paths::write_json(path, self)?;
        let _ = std::fs::remove_file(Self::activity_path_for(path));
        Ok(())
    }

    pub fn save_activity(&self) -> anyhow::Result<()> {
        self.save_activity_to(&self.storage_path.0.clone().unwrap_or_else(Self::path))
    }

    pub fn save_activity_to(&self, path: &Path) -> anyhow::Result<()> {
        self.activity_snapshot().save_to(&Self::activity_path_for(path))
    }

    pub fn is_liked(&self, track: &Track) -> bool {
        if self.liked.len() <= 4 {
            return self.liked.iter().any(|t| same_entry(t, track));
        }
        let len = self.liked.len();
        if let Ok(mut guard) = self.liked_cache.0.lock() {
            let idx = match guard.as_mut() {
                Some((n, idx)) if *n == len => idx,
                _ => {
                    let built = TrackIndex::from_tracks(&self.liked);
                    *guard = Some((len, built));
                    &mut guard.as_mut().unwrap().1
                }
            };
            return idx.contains(track);
        }
        self.liked.iter().any(|t| same_entry(t, track))
    }

    /// Call after changing liked-track metadata in place (e.g. remote sync).
    pub(crate) fn invalidate_liked_index(&mut self) {
        *self.liked_cache.0.get_mut().unwrap() = None;
    }

    pub fn toggle_like(&mut self, track: &Track) {
        if let Some(i) = self.liked.iter().position(|t| same_entry(t, track)) {
            self.liked.remove(i);
        } else {
            self.liked.insert(0, track.clone());
        }
        if let Ok(mut g) = self.liked_cache.0.lock() {
            *g = None;
        }
    }

    /// Counts a started song for its artists (taste profile).
    pub fn record_play(&mut self, track: &Track) {
        for a in crate::taste::credited(&track.artist) {
            *self.artist_plays.entry(a).or_default() += 1;
        }
        // Keep the map small: drop the rarest when it grows.
        if self.artist_plays.len() > 2000 {
            let mut v: Vec<(String, u32)> = std::mem::take(&mut self.artist_plays).into_iter().collect();
            v.sort_by(|a, b| b.1.cmp(&a.1));
            v.truncate(1500);
            self.artist_plays = v.into_iter().collect();
        }
    }

    /// Record a meaningful listen, once playback has reached the listening threshold.
    pub fn record_listen(&mut self, track: &Track, now: u64) {
        let key = format!("{}|{}", crate::dedupe::normalize_title(&track.title), crate::dedupe::normalize_artist(&track.artist));
        let score = self.song_activity.get(&key).map(|a| a.weight(now)).unwrap_or(0.0) + 1.0;
        self.song_activity.insert(key, SongActivity { track: track.clone(), score, last_listened: now });
        if self.song_activity.len() > 500 {
            let mut ranked: Vec<_> = std::mem::take(&mut self.song_activity).into_iter().collect();
            ranked.sort_by(|a,b| b.1.weight(now).total_cmp(&a.1.weight(now)).then(b.1.last_listened.cmp(&a.1.last_listened)));
            ranked.truncate(500);
            self.song_activity = ranked.into_iter().collect();
        }
    }

    /// Frequent and still-active songs, not the chronological recent list.
    pub fn active_tracks(&self, limit: usize, now: u64) -> Vec<Track> {
        let mut entries: Vec<_> = self.song_activity.values().map(|a| (a.weight(now), a)).filter(|(weight,_)| *weight >= 0.125).collect();
        entries.sort_by(|(wa,a),(wb,b)| wb.total_cmp(wa).then(b.last_listened.cmp(&a.last_listened)).then(a.track.title.cmp(&b.track.title)));
        entries.into_iter().take(limit).map(|(_,a)| a.track.clone()).collect()
    }

    /// Own playlists by recent listening to their songs (most active first);
    /// playlists with no recent activity are left out.
    pub fn active_playlists(&self, now: u64) -> Vec<String> {
        if self.song_activity.is_empty() {
            return Vec::new();
        }
        let mut ranked: Vec<(f64, &str)> = self
            .playlists
            .iter()
            .filter(|p| p.shared_by.is_none())
            .map(|p| {
                let w: f64 = p
                    .tracks
                    .iter()
                    .filter_map(|t| {
                        let key = format!("{}|{}", crate::dedupe::normalize_title(&t.title), crate::dedupe::normalize_artist(&t.artist));
                        self.song_activity.get(&key).map(|a| a.weight(now) as f64)
                    })
                    .sum();
                (w, p.id.as_str())
            })
            .filter(|(w, _)| *w >= 0.125)
            .collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        ranked.into_iter().map(|(_, id)| id.to_string()).collect()
    }

    /// Counts an early skip for the song's artists.
    pub fn record_skip(&mut self, track: &Track) {
        for a in crate::taste::credited(&track.artist) {
            *self.artist_skips.entry(a).or_default() += 1;
        }
        if self.artist_skips.len() > 2000 {
            let mut v: Vec<(String, u32)> = std::mem::take(&mut self.artist_skips).into_iter().collect();
            v.sort_by(|a, b| b.1.cmp(&a.1));
            v.truncate(1500);
            self.artist_skips = v.into_iter().collect();
        }
    }

    /// Songs in "recently played", newest first (artists are skipped).
    pub fn recent_tracks(&self) -> impl Iterator<Item = &Track> + '_ {
        self.recent.iter().filter_map(|i| match i {
            RecentItem::Song { track } => Some(track),
            RecentItem::Artist { .. } => None,
        })
    }

    /// Push to front, dedupe, cap at 50.
    pub fn push_recent(&mut self, track: Track) {
        self.push_recent_item(RecentItem::Song { track });
    }

    /// Remember that playback started from an artist's page, so the artist
    /// shows in "recently played" next to the songs.
    pub fn push_recent_artist(&mut self, name: &str, track: Option<Track>) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        self.push_recent_item(RecentItem::Artist { name: name.to_string(), track });
    }

    fn push_recent_item(&mut self, item: RecentItem) {
        self.recent.retain(|i| !i.same(&item));
        self.recent.insert(0, item);
        self.recent.truncate(RECENT_LIMIT);
    }

    /// Removes the song from "recently played" (the artist entry stays).
    pub fn remove_recent_song(&mut self, track: &Track) {
        self.recent.retain(|i| match i {
            RecentItem::Song { track: t } => !same_entry(t, track),
            RecentItem::Artist { .. } => true,
        });
    }

    /// Remember a song picked from search: front, deduped, capped.
    pub fn push_search(&mut self, track: Track) {
        self.push_search_item(SearchItem::Song { track });
    }

    /// Remember an artist opened from search.
    pub fn push_search_artist(&mut self, name: &str, track: Option<Track>) {
        self.push_search_item(SearchItem::Artist { name: name.trim().to_string(), track });
    }

    fn push_search_item(&mut self, item: SearchItem) {
        self.search_items.retain(|i| !i.same(&item));
        self.search_items.insert(0, item);
        self.search_items.truncate(SEARCH_HISTORY_LIMIT);
    }

    pub fn remove_search(&mut self, index: usize) {
        if index < self.search_items.len() {
            self.search_items.remove(index);
        }
    }

    pub fn is_hidden(&self, artist: &str) -> bool {
        self.hidden_artists.iter().any(|a| a.eq_ignore_ascii_case(artist.trim()))
    }

    /// Hides or shows an artist again. Returns true when now hidden.
    pub fn toggle_hidden(&mut self, artist: &str) -> bool {
        if let Some(i) = self.hidden_artists.iter().position(|a| a.eq_ignore_ascii_case(artist.trim())) {
            self.hidden_artists.remove(i);
            false
        } else {
            self.hidden_artists.push(artist.trim().to_string());
            true
        }
    }

    pub fn is_following(&self, name: &str) -> bool {
        self.followed.iter().any(|a| a.name.eq_ignore_ascii_case(name))
    }

    /// Follows or unfollows an artist. Returns true when now following.
    pub fn toggle_follow(&mut self, name: &str, track: Option<Track>) -> bool {
        if let Some(i) = self.followed.iter().position(|a| a.name.eq_ignore_ascii_case(name)) {
            self.followed.remove(i);
            false
        } else {
            self.followed.insert(0, FollowedArtist { name: name.trim().to_string(), track });
            true
        }
    }

    pub fn is_saved(&self, key: &ListKey) -> bool {
        self.saved_albums.iter().any(|k| k.same(key))
    }

    /// Saves or unsaves an album. Returns true when it is now saved.
    pub fn toggle_saved(&mut self, key: ListKey) -> bool {
        if let Some(i) = self.saved_albums.iter().position(|k| k.same(&key)) {
            self.saved_albums.remove(i);
            false
        } else {
            self.saved_albums.insert(0, key);
            true
        }
    }

    /// Remembers an opened list: front, deduped, capped.
    /// Short stable id of a list for `list_activity`.
    pub fn list_id(key: &ListKey) -> String {
        match key {
            ListKey::Liked => "liked".into(),
            ListKey::Recent => "recent".into(),
            ListKey::Downloads => "downloads".into(),
            ListKey::Playlist { id } => format!("pl:{id}"),
            ListKey::Album { title, artist, .. } => format!("al:{}|{}", title.to_lowercase(), artist.to_lowercase()),
            ListKey::ArtistLiked { artist } => format!("ar:{}", artist.to_lowercase()),
        }
    }

    /// Adds `amount` of use to a list (a song listened from it: 1, opening it: less).
    pub fn record_list_use(&mut self, key: &ListKey, amount: f64, now: u64) {
        self.record_use(&Self::list_id(key), amount, now);
    }

    /// [`Library::record_list_use`] by sidebar id (lists and artist pages).
    pub fn record_use(&mut self, id: &str, amount: f64, now: u64) {
        let id = id.to_string();
        let w = self.use_weight(&id, now) + amount;
        self.list_activity.insert(id, (w, now));
        if self.list_activity.len() > 200 {
            let mut v: Vec<_> = std::mem::take(&mut self.list_activity).into_iter().collect();
            v.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0));
            v.truncate(150);
            self.list_activity = v.into_iter().collect();
        }
    }

    /// Current use of a list, halving every two weeks without use.
    pub fn list_weight(&self, key: &ListKey, now: u64) -> f64 {
        self.use_weight(&Self::list_id(key), now)
    }

    /// Current use by sidebar id, halving every two weeks without use.
    pub fn use_weight(&self, id: &str, now: u64) -> f64 {
        self.list_activity
            .get(id)
            .map_or(0.0, |(s, at)| s * (-((now.saturating_sub(*at)) as f64) / (14.0 * 86400.0)).exp2())
    }

    /// Sidebar id of an artist page.
    pub fn artist_id(name: &str) -> String {
        format!("artist:{}", name.trim().to_lowercase())
    }

    /// Remembers an opened artist page (sidebar): front, deduped, capped.
    pub fn push_recent_artist_page(&mut self, name: &str, track: Option<Track>) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let track = track.or_else(|| self.recent_artists.iter().find(|a| a.name.eq_ignore_ascii_case(name)).and_then(|a| a.track.clone()));
        self.recent_artists.retain(|a| !a.name.eq_ignore_ascii_case(name));
        self.recent_artists.insert(0, FollowedArtist { name: name.to_string(), track });
        self.recent_artists.truncate(RECENT_ARTISTS_LIMIT);
    }

    pub fn is_sidebar_hidden(&self, id: &str) -> bool {
        self.sidebar_hidden.iter().any(|h| h == id)
    }

    /// Hides a list / artist from the sidebar or shows it again. Returns
    /// true when it is now hidden.
    pub fn toggle_sidebar_hidden(&mut self, id: &str) -> bool {
        if let Some(i) = self.sidebar_hidden.iter().position(|h| h == id) {
            self.sidebar_hidden.remove(i);
            false
        } else {
            self.sidebar_hidden.push(id.to_string());
            true
        }
    }

    pub fn push_recent_list(&mut self, key: ListKey) {
        self.recent_lists.retain(|k| !k.same(&key));
        self.recent_lists.insert(0, key);
        self.recent_lists.truncate(RECENT_LISTS_LIMIT);
    }

    pub fn create_playlist(&mut self, name: &str) -> &mut Playlist {
        self.playlists.push(Playlist {
            local_only: false,
            id: uuid::Uuid::new_v4().simple().to_string(),
            name: clean_playlist_name(name),
            tracks: vec![],
            origin: None,
            cover: None,
            auto_download: false,
            shared_by: None,
        });
        self.playlists.last_mut().expect("just pushed")
    }

    pub fn playlist(&self, id: &str) -> Option<&Playlist> {
        self.playlists.iter().find(|p| p.id == id)
    }

    pub fn playlist_mut(&mut self, id: &str) -> Option<&mut Playlist> {
        self.playlists.iter_mut().find(|p| p.id == id)
    }

    pub fn rename_playlist(&mut self, id: &str, name: &str) {
        if let Some(p) = self.playlist_mut(id) {
            p.name = clean_playlist_name(name);
        }
    }

    pub fn set_playlist_cover(&mut self, id: &str, cover: Option<String>) {
        if let Some(p) = self.playlist_mut(id) {
            p.cover = cover;
        }
    }

    pub fn delete_playlist(&mut self, id: &str) {
        self.checkpoint_playlists();
        if let Some(p) = self.playlist(id).cloned() { self.playlist_trash.insert(id.to_string(), p); }
        self.playlists.retain(|p| p.id != id);
        self.recent_lists.retain(|k| k != &ListKey::Playlist { id: id.to_string() });
    }

    pub fn checkpoint_playlists(&mut self) {
        use sha2::{Digest, Sha256};
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        for p in &self.playlists {
            let fresh = !self.playlist_membership.contains_key(&p.id);
            let membership = self.playlist_membership.entry(p.id.clone()).or_default();
            let ids: std::collections::BTreeSet<_> = p.tracks.iter().map(crate::sync::track_id).collect();
            for (id, op) in membership.iter_mut() {
                let present = ids.contains(id);
                if present != op.present {
                    *op = PlaylistMembership { at: now.max(op.at.saturating_add(1)), device: crate::server_client::install_id(), present };
                }
            }
            for id in ids {
                membership.entry(id).or_insert_with(|| PlaylistMembership { at: if fresh { 0 } else { now }, device: crate::server_client::install_id(), present: true });
            }
            let bytes = serde_json::to_vec(p).unwrap_or_default();
            let key = format!("{:x}", Sha256::digest(bytes));
            self.playlist_versions.entry(key).or_insert_with(|| PlaylistRevision {
                playlist: p.clone(), saved_at: paths::now_unix(),
            });
        }
    }

    pub fn restore_playlist(&mut self, id: &str) -> bool {
        let Some(p) = self.playlist_trash.remove(id) else { return false };
        if self.playlist(id).is_none() { self.playlists.push(p); }
        true
    }

    pub fn restore_playlist_version(&mut self, revision: &str) -> bool {
        let Some(p) = self.playlist_versions.get(revision).map(|v| v.playlist.clone()) else { return false };
        self.checkpoint_playlists();
        self.playlist_trash.remove(&p.id);
        if let Some(current) = self.playlist_mut(&p.id) { *current = p; } else { self.playlists.push(p); }
        true
    }

    /// Appends a song unless the playlist already has it. Returns `true` if added.
    pub fn add_to_playlist(&mut self, id: &str, track: Track) -> bool {
        let Some(p) = self.playlist_mut(id) else { return false };
        if p.tracks.len() >= playlist_track_limit() || p.tracks.iter().any(|t| same_entry(t, &track)) {
            return false;
        }
        p.tracks.push(track);
        true
    }

    pub fn remove_from_playlist(&mut self, id: &str, index: usize) {
        if let Some(p) = self.playlist_mut(id) {
            if index < p.tracks.len() {
                p.tracks.remove(index);
            }
        }
    }

    pub fn move_in_playlist(&mut self, id: &str, from: usize, to: usize) {
        if let Some(p) = self.playlist_mut(id) {
            if from < p.tracks.len() && to < p.tracks.len() {
                let t = p.tracks.remove(from);
                p.tracks.insert(to, t);
            }
        }
    }

    /// Saves imported tracks as a playlist. If a playlist with the same
    /// `origin` already exists, only songs that are not in it yet are added
    /// (re-import). Returns the playlist and how many songs were added.
    pub fn save_import(&mut self, name: &str, origin: Option<&str>, tracks: Vec<Track>) -> (&mut Playlist, usize) {
        let existing = origin.and_then(|o| self.playlists.iter().position(|p| p.origin.as_deref() == Some(o)));
        let idx = match existing {
            Some(i) => i,
            None => {
                let p = self.create_playlist(name);
                p.origin = origin.map(str::to_string);
                self.playlists.len() - 1
            }
        };
        let p = &mut self.playlists[idx];
        let mut seen = TrackIndex::from_tracks(&p.tracks);
        let mut added = 0;
        for t in tracks {
            if p.tracks.len() >= playlist_track_limit() { break; }
            if !seen.contains(&t) {
                seen.insert(&t);
                p.tracks.push(t);
                added += 1;
            }
        }
        (&mut self.playlists[idx], added)
    }

    /// Saves imported albums after the ones already saved (same title +
    /// artist is skipped). Returns how many were added.
    pub fn save_albums(&mut self, albums: impl IntoIterator<Item = (String, String, Vec<Track>)>) -> usize {
        let mut added = 0;
        for (title, artist, tracks) in albums {
            let key = ListKey::Album { title, artist, tracks };
            if !tracks_empty(&key) && !self.is_saved(&key) {
                self.saved_albums.push(key);
                added += 1;
            }
        }
        added
    }

    /// Follows imported artists after the ones already followed (no
    /// duplicates). Returns how many were added.
    pub fn follow_all(&mut self, names: impl IntoIterator<Item = String>) -> usize {
        let mut added = 0;
        for name in names {
            let name = name.trim();
            if !name.is_empty() && !self.is_following(name) {
                self.followed.push(FollowedArtist { name: name.to_string(), track: None });
                added += 1;
            }
        }
        added
    }

    /// Adds imported tracks to Liked Songs (no duplicates). Returns how many were added.
    pub fn like_all(&mut self, tracks: Vec<Track>) -> usize {
        let mut seen = TrackIndex::from_tracks(&self.liked);
        let mut added = 0;
        for t in tracks {
            if !seen.contains(&t) {
                seen.insert(&t);
                self.liked.push(t);
                added += 1;
            }
        }
        if added > 0 {
            if let Ok(mut g) = self.liked_cache.0.lock() {
                *g = None;
            }
        }
        added
    }

    /// Lyrics offset in ms for a track (default 0), stepped by ±100 in the UI.
    pub fn lyrics_offset(&self, track: &Track) -> i64 {
        self.lyrics_offsets.get(&track_key(track)).copied().unwrap_or(0)
    }

    pub fn set_lyrics_offset(&mut self, track: &Track, offset_ms: i64) {
        let key = track_key(track);
        if offset_ms == 0 {
            self.lyrics_offsets.remove(&key);
        } else {
            self.lyrics_offsets.insert(key, offset_ms);
        }
    }
}

/// Same library entry: same provider id, or the same song by title/artist/duration.
pub(crate) fn same_entry(a: &Track, b: &Track) -> bool {
    (a.source == b.source && a.id == b.id)
        || a.external_ids.get(&b.source).map(|id| id == &b.id).unwrap_or(false)
        || b.external_ids.get(&a.source).map(|id| id == &a.id).unwrap_or(false)
        || same_song(a, b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dedupe::test_track;
    use glyph_protocol::ProviderId;

    fn tr(id: &str, title: &str) -> Track {
        test_track(ProviderId::Audius, id, title, "Band", Some(180_000))
    }

    #[test]
    fn activity_favors_repeated_current_listens_and_preserves_recent_order() {
        let mut lib = Library::default();
        let now = 30 * 86400;
        let favorite = tr("favorite","Favorite");
        let latest = tr("latest","Latest");
        lib.push_recent(favorite.clone()); lib.push_recent(latest.clone());
        for _ in 0..5 { lib.record_listen(&favorite,now-86400); }
        lib.record_listen(&latest,now);
        assert_eq!(lib.active_tracks(2,now),vec![favorite.clone(),latest.clone()]);
        assert_eq!(lib.recent_tracks().cloned().collect::<Vec<_>>(),vec![latest.clone(),favorite.clone()]);
        let hot = tr("hot","New favorite");
        for _ in 0..3 { lib.record_listen(&hot,now+28*86400); }
        assert_eq!(lib.active_tracks(1,now+28*86400),vec![hot]);
        let mut alternate = favorite.clone(); alternate.source=ProviderId::Deezer; alternate.id="alternate".into();
        let count=lib.song_activity.len(); lib.record_listen(&alternate,now+28*86400);
        assert_eq!(lib.song_activity.len(),count,"same song on another provider is one activity entry");
        let encoded=serde_json::to_vec(&lib).unwrap();
        let decoded: Library = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.active_tracks(5,now+28*86400),lib.active_tracks(5,now+28*86400));
        for (key,value) in &lib.song_activity { assert!((decoded.song_activity[key].score-value.score).abs()<1e-12); }
    }

    #[test]
    fn follow_toggle_is_case_insensitive() {
        let mut lib = Library::default();
        assert!(lib.toggle_follow("Daft Punk", None));
        assert!(lib.is_following("daft punk"));
        assert!(!lib.toggle_follow("DAFT PUNK", None));
        assert!(lib.followed.is_empty());
    }

    #[test]
    fn saved_albums_and_recent_lists() {
        let mut lib = Library::default();
        let a = ListKey::Album { title: "Discovery".into(), artist: "Daft Punk".into(), tracks: vec![tr("1", "One")] };
        let same = ListKey::Album { title: "discovery".into(), artist: "daft punk".into(), tracks: vec![] };
        assert!(lib.toggle_saved(a.clone()));
        assert!(lib.is_saved(&same));
        assert!(!lib.toggle_saved(same));
        assert!(lib.saved_albums.is_empty());
        for i in 0..(RECENT_LISTS_LIMIT + 2) {
            lib.push_recent_list(ListKey::Playlist { id: i.to_string() });
        }
        lib.push_recent_list(ListKey::Playlist { id: "5".into() });
        assert_eq!(lib.recent_lists.len(), RECENT_LISTS_LIMIT);
        assert_eq!(lib.recent_lists[0], ListKey::Playlist { id: "5".into() });
        lib.playlists.push(Playlist { local_only: false, id: "5".into(), name: "x".into(), tracks: vec![], origin: None, cover: None, auto_download: false, shared_by: None });
        lib.delete_playlist("5");
        assert!(!lib.recent_lists.contains(&ListKey::Playlist { id: "5".into() }));
    }

    fn song_title(i: &SearchItem) -> &str {
        match i {
            SearchItem::Song { track } => &track.title,
            SearchItem::Artist { name, .. } => name,
        }
    }

    #[test]
    fn search_history_front_dedupe_cap_remove() {
        let mut lib = Library::default();
        for i in 0..25 {
            lib.push_search(tr(&i.to_string(), &format!("Song {i}")));
        }
        assert_eq!(lib.search_items.len(), SEARCH_HISTORY_LIMIT);
        assert_eq!(song_title(&lib.search_items[0]), "Song 24");
        lib.push_search(tr("20", "Song 20"));
        assert_eq!(song_title(&lib.search_items[0]), "Song 20");
        assert_eq!(lib.search_items.iter().filter(|t| song_title(t) == "Song 20").count(), 1);
        lib.remove_search(0);
        assert_eq!(song_title(&lib.search_items[0]), "Song 24");
        lib.remove_search(999);
        lib.push_search_artist("Daft Punk", None);
        lib.push_search_artist("daft punk", None);
        assert_eq!(lib.search_items.iter().filter(|i| matches!(i, SearchItem::Artist { .. })).count(), 1);
    }

    #[test]
    fn hidden_artists_and_history_migration() {
        let mut lib = Library::default();
        assert!(lib.toggle_hidden("Spam Band"));
        assert!(lib.is_hidden("spam band "));
        assert!(!lib.toggle_hidden("SPAM BAND"));
        assert!(lib.hidden_artists.is_empty());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.json");
        let mut old = Library::default();
        old.search_history = vec![tr("1", "Old")];
        old.save_to(&path).unwrap();
        let back = Library::load_from(&path).unwrap();
        assert!(back.search_history.is_empty());
        assert_eq!(song_title(&back.search_items[0]), "Old");
    }

    #[test]
    fn like_toggle_and_recent_cap() {
        let mut lib = Library::default();
        let t = tr("1", "Song");
        assert!(!lib.is_liked(&t));
        lib.toggle_like(&t);
        assert!(lib.is_liked(&t));
        // same song from another provider counts as liked
        let other = test_track(ProviderId::Deezer, "d9", "Song (Remastered)", "Band", Some(181_000));
        assert!(lib.is_liked(&other));
        lib.toggle_like(&t);
        assert!(!lib.is_liked(&t));

        for i in 0..60 {
            lib.push_recent(tr(&i.to_string(), &format!("Song {i}")));
        }
        assert_eq!(lib.recent.len(), 50);
        assert_eq!(lib.recent[0].track().unwrap().id, "59");
        lib.push_recent(tr("30", "Song 30"));
        assert_eq!(lib.recent.len(), 50);
        assert_eq!(lib.recent[0].track().unwrap().id, "30");
        assert_eq!(lib.recent_tracks().filter(|t| t.id == "30").count(), 1);
    }

    #[test]
    fn recent_keeps_artists_and_migrates_plain_songs() {
        let mut lib = Library::default();
        let first = tr("1", "One");
        lib.push_recent(first.clone());
        lib.push_recent_artist("Daft Punk", Some(first.clone()));
        assert!(matches!(lib.recent[0], RecentItem::Artist { .. }));
        assert_eq!(lib.recent.len(), 2);
        assert_eq!(lib.recent_tracks().count(), 1);
        // Playing a song from the artist again moves the artist back to front.
        lib.push_recent(tr("2", "Two"));
        lib.push_recent_artist("daft punk", Some(first.clone()));
        assert_eq!(lib.recent.len(), 3);
        assert_eq!(lib.recent[0].artist_name(), Some("daft punk"));
        lib.remove_recent_song(&first);
        assert_eq!(lib.recent_tracks().count(), 1);

        // Older files stored plain tracks, without a `kind`.
        let legacy = serde_json::json!({ "liked": [], "recent": [first], "playlists": [] });
        let decoded: Library = serde_json::from_value(legacy).unwrap();
        assert_eq!(decoded.recent_tracks().cloned().collect::<Vec<_>>(), vec![tr("1", "One")]);
        // The new form survives a round trip.
        let encoded = serde_json::to_value(&lib).unwrap();
        let decoded: Library = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.recent, lib.recent);
    }

    #[test]
    fn imported_albums_and_artists() {
        let mut lib = Library::default();
        lib.toggle_follow("Daft Punk", None);
        let n = lib.follow_all(vec!["daft punk".into(), "Air".into(), " ".into(), "Justice".into(), "Air".into()]);
        assert_eq!(n, 2);
        let names: Vec<&str> = lib.followed.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Daft Punk", "Air", "Justice"]);
        let n = lib.save_albums(vec![
            ("RAM".to_string(), "Daft Punk".to_string(), vec![tr("1", "a")]),
            ("ram".to_string(), "daft punk".to_string(), vec![tr("2", "b")]),
            ("Empty".to_string(), "X".to_string(), vec![]),
        ]);
        assert_eq!(n, 1);
        assert_eq!(lib.saved_albums.len(), 1);
    }

    #[test]
    fn playlist_limit_covers_add_and_reimport() {
        let mut lib = Library::default();
        let (p, _) = lib.save_import("Full", Some("origin"), vec![]);
        p.tracks = (0..MAX_PLAYLIST_TRACKS).map(|i| tr(&i.to_string(), &format!("Song {i}"))).collect();
        let id = p.id.clone();
        assert!(!lib.add_to_playlist(&id, tr("extra", "Extra")));
        let (p, added) = lib.save_import("Full", Some("origin"), vec![tr("extra", "Extra")]);
        assert_eq!(added, 0);
        assert_eq!(p.tracks.len(), MAX_PLAYLIST_TRACKS);
    }

    #[test]
    fn playlists_crud_and_reimport() {
        let mut lib = Library::default();
        let id = lib.create_playlist("  Mix ").id.clone();
        assert_eq!(lib.playlist(&id).unwrap().name, "Mix");
        lib.rename_playlist(&id, "Better");
        assert!(lib.add_to_playlist(&id, tr("1", "A")));
        assert!(!lib.add_to_playlist(&id, tr("1", "A")));
        assert!(lib.add_to_playlist(&id, tr("2", "B")));
        lib.move_in_playlist(&id, 1, 0);
        assert_eq!(lib.playlist(&id).unwrap().tracks[0].id, "2");
        lib.remove_from_playlist(&id, 0);
        assert_eq!(lib.playlist(&id).unwrap().tracks.len(), 1);
        lib.delete_playlist(&id);
        assert!(lib.playlists.is_empty());

        let (p, added) = lib.save_import("Imp", Some("https://x/pl"), vec![tr("1", "A"), tr("2", "B")]);
        assert_eq!((p.tracks.len(), added), (2, 2));
        let (p, added) = lib.save_import("Imp", Some("https://x/pl"), vec![tr("2", "B"), tr("3", "C")]);
        assert_eq!((p.tracks.len(), added), (3, 1));
        assert_eq!(lib.playlists.len(), 1);
    }

    #[test]
    fn offsets_and_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.json");
        assert_eq!(Library::load_from(&path).unwrap(), Library::default());

        let mut lib = Library::default();
        let t = tr("1", "Song");
        lib.set_lyrics_offset(&t, -300);
        assert_eq!(lib.lyrics_offset(&t), -300);
        lib.toggle_like(&t);
        lib.save_to(&path).unwrap();
        let back = Library::load_from(&path).unwrap();
        assert_eq!(back, lib);
        let mut back = back;
        back.set_lyrics_offset(&t, 0);
        assert_eq!(back.lyrics_offset(&t), 0);

        std::fs::write(&path, b"{ nope").unwrap();
        assert!(Library::load_from(&path).is_err());
        assert!(path.with_extension("json.bak").exists());
    }

    #[test]
    fn captured_library_and_activity_saves_keep_their_profile_path() {
        let dir = tempfile::tempdir().unwrap();
        let guest_path = dir.path().join("guest/library.json");
        let account_path = dir.path().join("account/library.json");
        let mut guest = Library::load_from(&guest_path).unwrap();
        guest.toggle_like(&tr("guest", "Guest song"));
        let pending_guest = guest.clone();
        let mut account = Library::load_from(&account_path).unwrap();
        account.toggle_like(&tr("account", "Account song"));
        account.save_to(&account.storage_path.0.clone().unwrap()).unwrap();
        pending_guest.save_to(&pending_guest.storage_path.0.clone().unwrap()).unwrap();
        assert_eq!(Library::load_from(&guest_path).unwrap().liked[0].id, "guest");
        assert_eq!(Library::load_from(&account_path).unwrap().liked[0].id, "account");
        assert_ne!(guest.storage_path.0, account.storage_path.0);
    }

    #[test]
    fn activity_sidecar_overlays_and_cleans_up_on_full_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.json");
        let mut lib = Library::default();
        let liked = tr("1", "Liked");
        let played = tr("2", "Played");
        lib.toggle_like(&liked);
        lib.save_to(&path).unwrap();

        lib.push_recent(played.clone());
        lib.record_play(&played);
        lib.record_listen(&played, 1000);
        lib.save_activity_to(&path).unwrap();
        assert!(Library::activity_path_for(&path).exists());

        let loaded = Library::load_from(&path).unwrap();
        assert!(loaded.is_liked(&liked));
        assert_eq!(loaded.recent_tracks().cloned().collect::<Vec<_>>(), vec![played]);
        assert_eq!(loaded.artist_plays.get("band"), Some(&1));

        loaded.save_to(&path).unwrap();
        assert!(!Library::activity_path_for(&path).exists());
    }
}
