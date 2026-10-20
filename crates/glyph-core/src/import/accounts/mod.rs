//! Moving a whole library to Glyph ("Transfer music"): pick a service, hand
//! Glyph what it can read, choose what comes over (liked songs, playlists,
//! saved albums, followed artists), and Glyph copies the lists and their
//! metadata. Songs are matched to legal full sources like any import;
//! nothing is downloaded.
//!
//! Nobody registers an app anywhere and nobody logs in:
//! - Spotify, YouTube Music: paste what was copied in their app (Spotify:
//!   Ctrl+A, Ctrl+C in Liked Songs or any playlist gives one song link per
//!   line; playlist links work too) or a Google Takeout file ([`paste`]).
//! - Deezer, SoundCloud: a public profile. SoundCloud goes through the Glyph
//!   server (its key) unless the app has an own SoundCloud key.
//! - Last.fm: the user name (loved tracks through the server's key).
//!
//! The run itself is `Backend::migrate`; [`MigrationState`] remembers which
//! lists are done, so a stopped move continues where it left off.

pub mod deezer;
pub mod lastfm;
pub mod paste;
pub mod soundcloud;

use crate::import::files::ParsedList;
use crate::import::keyless::SongLink;
use crate::{ImportInput, ImportResult};
use anyhow::Result;
use async_trait::async_trait;
use glyph_protocol::Track;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Service {
    Spotify,
    YouTube,
    SoundCloud,
    Deezer,
    LastFm,
}

/// What the user hands over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connect {
    /// Copied song / playlist links (from the clipboard), plus files
    /// (`takeout`: Google Takeout CSVs are the way for private lists).
    Paste { takeout: bool },
    /// A public profile link or id.
    Profile,
    /// A user name.
    UserName,
}

impl Service {
    pub const ALL: [Service; 5] = [Service::Spotify, Service::YouTube, Service::SoundCloud, Service::Deezer, Service::LastFm];

    pub fn name(self) -> &'static str {
        match self {
            Service::Spotify => "Spotify",
            Service::YouTube => "YouTube Music",
            Service::SoundCloud => "SoundCloud",
            Service::Deezer => "Deezer",
            Service::LastFm => "Last.fm",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Service::Spotify => "spotify",
            Service::YouTube => "youtube",
            Service::SoundCloud => "soundcloud",
            Service::Deezer => "deezer",
            Service::LastFm => "lastfm",
        }
    }

    pub fn connect(self) -> Connect {
        match self {
            Service::Spotify => Connect::Paste { takeout: false },
            Service::YouTube => Connect::Paste { takeout: true },
            Service::SoundCloud | Service::Deezer => Connect::Profile,
            Service::LastFm => Connect::UserName,
        }
    }

    /// Where the service hands out a copy of the user's data (private lists).
    pub fn export_url(self) -> Option<&'static str> {
        match self {
            Service::YouTube => Some("https://takeout.google.com/settings/takeout/custom/youtube"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Liked / favourite / loved songs -> Glyph's Liked Songs.
    Liked,
    /// One playlist -> a Glyph playlist.
    Playlist,
    /// All saved albums -> the album library.
    Albums,
    /// All followed artists -> Glyph's "Following".
    Artists,
}

/// One thing that can come over.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Collection {
    /// Stable within the source (`liked`, `albums`, `artists`, `pl:<id>`).
    pub id: String,
    pub kind: Kind,
    /// Empty: not known before reading (pasted links); shown by `hint`.
    pub name: String,
    /// Songs / albums / artists, when the service says.
    pub count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
    /// The playlist's public address: stored as the Glyph playlist's origin,
    /// so a second move (or a pasted link) only adds new songs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// Ticked in the overview by default.
    pub default_on: bool,
    /// Playlist made by someone else (followed / liked playlist).
    #[serde(default)]
    pub followed: bool,
    /// Shown while the name is unknown (`spotify/playlist · 37i9dQ…`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hint: String,
}

impl Collection {
    fn new(id: &str, kind: Kind, count: Option<usize>) -> Self {
        Self { id: id.into(), kind, name: String::new(), count, cover: None, origin: None, default_on: true, followed: false, hint: String::new() }
    }
    pub fn liked(count: Option<usize>) -> Self {
        Self::new("liked", Kind::Liked, count)
    }
    pub fn albums(count: Option<usize>) -> Self {
        Self::new("albums", Kind::Albums, count)
    }
    pub fn artists(count: Option<usize>) -> Self {
        Self::new("artists", Kind::Artists, count)
    }
    pub fn playlist(id: &str, name: &str, count: Option<usize>, cover: Option<String>, origin: Option<String>) -> Self {
        Self { name: name.to_string(), cover, origin, ..Self::new(&format!("pl:{id}"), Kind::Playlist, count) }
    }
    /// The service's own id of a playlist collection.
    pub fn playlist_id(&self) -> &str {
        self.id.strip_prefix("pl:").unwrap_or(&self.id)
    }
}

/// What the account holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Overview {
    /// Display name of the account.
    pub account: String,
    pub collections: Vec<Collection>,
    /// Hints for the result page (keys of `migrate.note.*`).
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedAlbum {
    pub title: String,
    pub artist: String,
    pub tracks: Vec<Track>,
}

/// One collection, read from the service.
#[derive(Debug, Clone, PartialEq)]
pub enum Fetched {
    Tracks(Vec<Track>),
    /// Songs of a list whose name was only known after reading it.
    Named(String, Vec<Track>),
    Albums(Vec<SavedAlbum>),
    Artists(Vec<String>),
}

#[async_trait]
pub trait LibrarySource: Send + Sync {
    fn service(&self) -> Service;
    async fn overview(&self) -> Result<Overview>;
    async fn fetch(&self, c: &Collection) -> Result<Fetched>;
}

/// Reads links and files the way a pasted link is imported (server first,
/// then the app's keys, then the ways without keys). Given by the backend.
#[async_trait]
pub trait LinkReader: Send + Sync {
    async fn list(&self, input: ImportInput) -> Result<ParsedList>;
    /// Metadata of single song links, in order.
    async fn songs(&self, songs: Vec<SongLink>) -> Vec<Track>;
}

/// What the user handed over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Login {
    /// Profile link, id or user name.
    Public(String),
    /// Copied text with links, and picked files.
    Pasted { text: String, files: Vec<PathBuf> },
}

/// What a source needs besides the login.
pub struct SourceCtx {
    pub http: reqwest::Client,
    pub keys: crate::config::ApiKeys,
    pub lastfm: Option<Arc<crate::providers::lastfm::LastFm>>,
    pub deezer: Option<Arc<crate::providers::deezer::Deezer>>,
    pub reader: Arc<dyn LinkReader>,
}

pub fn source(service: Service, login: Login, ctx: SourceCtx) -> Result<Arc<dyn LibrarySource>> {
    use anyhow::anyhow;
    let public = |l: &Login| match l {
        Login::Public(s) if !s.trim().is_empty() => Ok(s.trim().to_string()),
        _ => Err(anyhow!("enter your profile link or name")),
    };
    let src: Arc<dyn LibrarySource> = match (service, login) {
        (_, Login::Pasted { text, files }) => {
            let lib = paste::PasteLibrary::new(service, &text, files, ctx.reader);
            if lib.is_empty() {
                return Err(anyhow!("no song or playlist links found in what was copied"));
            }
            Arc::new(lib)
        }
        (Service::Deezer, login) => {
            let dz = ctx.deezer.unwrap_or_else(|| Arc::new(crate::providers::deezer::Deezer::new(ctx.http.clone())));
            Arc::new(deezer::DeezerLibrary::new(dz, deezer::user_id(&public(&login)?)?))
        }
        (Service::SoundCloud, login) => {
            let sc = ctx
                .keys
                .soundcloud_client_id
                .clone()
                .filter(|k| !k.trim().is_empty())
                .map(|id| crate::providers::soundcloud::SoundCloud::new(ctx.http.clone(), id));
            Arc::new(soundcloud::SoundCloudLibrary::new(sc, soundcloud::profile_url(&public(&login)?)?, ctx.reader))
        }
        (Service::LastFm, login) => Arc::new(lastfm::LastFmLibrary::new(ctx.lastfm, lastfm::user_name(&public(&login)?), ctx.reader)),
        (other, _) => return Err(anyhow!("copy the songs in {} and paste them here", other.name())),
    };
    Ok(src)
}

// ------------------------------------------------------------ run

/// What the move reports while it runs.
#[derive(Debug, Clone, PartialEq)]
pub enum MigrationEvent {
    /// Starting collection `index` of `total`.
    Collection { index: usize, total: usize, collection: Collection },
    /// Matching progress inside the current collection.
    Progress { done: usize, total: usize },
    Done { collection: Collection, result: Migrated },
    Failed { collection: Collection, error: String },
    /// All collections handled (or cancelled).
    Finished { cancelled: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Migrated {
    Tracks(ImportResult),
    Albums(Vec<SavedAlbum>),
    Artists(Vec<String>),
}

/// `matched 2814/2990 (94%)`-style totals of a finished move.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Totals {
    pub songs: usize,
    pub matched: usize,
    pub unsure: usize,
    pub liked: usize,
    pub playlists: usize,
    pub albums: usize,
    pub artists: usize,
    pub failed: usize,
}

impl Totals {
    pub fn add(&mut self, c: &Collection, r: &Migrated) {
        match r {
            Migrated::Tracks(res) => {
                self.songs += res.sure.len() + res.unsure.len() + res.not_found.len();
                self.matched += res.sure.len() + res.unsure.len();
                self.unsure += res.unsure.len();
                match c.kind {
                    Kind::Liked => self.liked += res.ordered.len(),
                    _ => self.playlists += 1,
                }
            }
            Migrated::Albums(a) => self.albums += a.len(),
            Migrated::Artists(a) => self.artists += a.len(),
        }
    }

    pub fn percent(&self) -> usize {
        if self.songs == 0 { 100 } else { self.matched * 100 / self.songs }
    }
}

// ------------------------------------------------------------ resume

/// A move in progress, saved after every collection (`migration.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MigrationState {
    pub service: Service,
    /// Profile / user name typed in (public services), to reconnect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Copied links (Spotify / YouTube Music), to read them again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pasted: Option<String>,
    /// Picked files (Takeout CSVs, exports).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<PathBuf>,
    pub account: String,
    /// Collections picked, in order.
    pub selected: Vec<Collection>,
    /// Ids of the collections already saved.
    pub done: Vec<String>,
    pub started: u64,
}

impl MigrationState {
    pub fn path() -> PathBuf {
        crate::paths::data_dir().join("migration.json")
    }

    pub fn load() -> Option<Self> {
        let text = std::fs::read_to_string(Self::path()).ok()?;
        serde_json::from_str::<Self>(&text).ok().filter(|s| !s.remaining().is_empty())
    }

    pub fn save(&self) -> Result<()> {
        crate::paths::write_atomic(&Self::path(), serde_json::to_string_pretty(self)?.as_bytes())
    }

    pub fn clear() {
        let _ = std::fs::remove_file(Self::path());
    }

    /// Collections not saved yet.
    pub fn remaining(&self) -> Vec<Collection> {
        self.selected.iter().filter(|c| !self.done.contains(&c.id)).cloned().collect()
    }

    pub fn mark_done(&mut self, id: &str) {
        if !self.done.iter().any(|d| d == id) {
            self.done.push(id.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dedupe::test_track;
    use crate::ImportMatch;
    use glyph_protocol::ProviderId;

    #[test]
    fn resume_state_round_trip() {
        let mut s = MigrationState {
            service: Service::Spotify,
            profile: None,
            pasted: Some("https://open.spotify.com/track/abc".into()),
            files: vec![],
            account: "me".into(),
            selected: vec![Collection::liked(Some(3)), Collection::playlist("1", "Road", Some(2), None, None), Collection::artists(Some(9))],
            done: vec![],
            started: 1,
        };
        s.mark_done("liked");
        s.mark_done("liked");
        assert_eq!(s.done, ["liked"]);
        let ids: Vec<String> = s.remaining().into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["pl:1", "artists"]);
        let text = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<MigrationState>(&text).unwrap(), s);
        assert_eq!(s.selected[1].playlist_id(), "1");
    }

    #[test]
    fn totals() {
        let m = |id: &str| ImportMatch { original: test_track(ProviderId::Spotify, id, "t", "a", None), resolved: None };
        let res = ImportResult { name: "x".into(), origin: None, sure: vec![m("1"), m("2")], unsure: vec![m("3")], not_found: vec![m("4")], ordered: vec![test_track(ProviderId::Spotify, "1", "t", "a", None); 4] };
        let mut t = Totals::default();
        t.add(&Collection::liked(None), &Migrated::Tracks(res.clone()));
        t.add(&Collection::playlist("p", "P", None, None, None), &Migrated::Tracks(res));
        t.add(&Collection::artists(None), &Migrated::Artists(vec!["A".into(), "B".into()]));
        assert_eq!((t.songs, t.matched, t.unsure, t.liked, t.playlists, t.artists), (8, 6, 2, 4, 1, 2));
        assert_eq!(t.percent(), 75);
    }
}
