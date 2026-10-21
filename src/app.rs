//! Application state and logic (`Glyph` entity). Rendering lives in `ui/`.
//!
//! Every call into glyph-core / glyph-audio goes through `crate::guard`, so a
//! backend that is not ready yet (or panics) degrades to `[ BACKEND OFFLINE ]`
//! instead of taking the window down.

use crate::guard::{guard, guard_async, guard_res};
use crate::i18n::{t, tf};
use crate::ui::input::{InputEvent, TextInput};
use crate::visuals::{ascii, Palette, Preset};
use crate::visuals::glow::GlowView;
use glyph_audio::{
    MediaKeyEvent, MediaKeys, MiniPlayerOptions, PlayerController, PlayerEvent, PlayerState,
    Status,
};
use glyph_core::{
    flume_like, Backend, BackendMode, Config, Library, Lyrics, Queue, RecentItem, RepeatMode, UpdateInfo,
    UpdateStatus,
};
use glyph_protocol::{ProviderId, SearchEvent, Track};
use gpui::{App, AppContext, Context, Entity, FocusHandle, SharedString, Subscription, Task, Window};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

mod boot;
mod poll;
mod player_state;
mod search;
mod playback;
mod library;
mod lyrics;
mod lists;
mod downloads;
mod navigation;
mod artist;
mod menus;
mod settings;
mod covers;
mod import;
mod net_devices;
mod account_devices;
mod account_library;
mod jam;
pub mod sleep;
pub use jam::{reaction_label as jam_reaction_label, Jam, JamUi};
pub use import::LinkImport;

/// Typing pause before a search goes out (Enter searches at once).
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(500);
/// Quiet time after the last settings change before the file is written.
const CONFIG_SAVE_DELAY: Duration = Duration::from_millis(700);
/// Quiet time after listening/navigation activity before the activity sidecar is written.
const ACTIVITY_SAVE_DELAY: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Home,
    Search,
    Library,
    NowPlaying,
    Settings,
    /// A song list (Liked, Recent, a playlist or an album): `Glyph::open_list`.
    List,
    /// Artist profile: `Glyph::open_artist`.
    Artist,
    /// A user's profile: `Glyph::open_profile`.
    Profile,
    /// One Home section on its own ("Show all"): `Glyph::section`.
    Section,
    /// All releases of one artist ("Show all" of the artist's albums):
    /// `Glyph::open_artist_releases`.
    Releases,
    /// All similar artists of one artist ("Show all" of the artist's similar
    /// artists): `Glyph::open_artist_similar`.
    Similar,
    /// Listening statistics (opt-in): `Glyph::open_stats`.
    Stats,
}

/// Which song list the list view shows.
#[derive(Debug, Clone, PartialEq)]
pub enum ListRef {
    Liked,
    Recent,
    User(String),
    Album { title: String, artist: String, tracks: Vec<Track> },
    /// Liked songs by one artist.
    ArtistLiked(String),
    /// Songs saved for offline use.
    Downloads,
}

/// "Add songs" side panel of one of the user's playlists: suggestions,
/// liked / recent songs, or a search.
pub struct AddSongs {
    pub id: String,
    pub open: bool,
    /// 0 suggested, 1 liked, 2 recently played.
    pub tab: u8,
    pub sugg: Vec<Track>,
    pub sugg_loading: bool,
    pub query: String,
    pub results: Vec<Track>,
    pub searching: bool,
    seq: u64,
    task: Option<Task<()>>,
}

/// "New playlist" / "Rename" dialog.
#[derive(Debug, Clone)]
pub struct NameDialog {
    /// Songs to add to the new playlist.
    pub tracks: Vec<Track>,
    /// Playlist id when renaming.
    pub rename: Option<String>,
    /// Naming an equalizer preset instead of a playlist.
    pub eq_preset: bool,
    pub initial_playlist: Option<(String, Option<String>, bool, Option<bool>)>,
}

impl ListRef {
    /// Id under which the list's sort order is saved (`None`: not sortable;
    /// Recently played mixes songs and artists in time order).
    pub fn sort_id(&self) -> Option<String> {
        match self {
            ListRef::Liked => Some("liked".into()),
            ListRef::Recent => None,
            ListRef::User(id) => Some(format!("playlist:{id}")),
            ListRef::Album { title, artist, .. } => Some(format!("album:{artist}:{title}")),
            ListRef::ArtistLiked(a) => Some(format!("liked:{a}")),
            ListRef::Downloads => Some("downloads".into()),
        }
    }

    pub fn key(&self) -> glyph_core::ListKey {
        use glyph_core::ListKey as K;
        match self {
            ListRef::Liked => K::Liked,
            ListRef::Recent => K::Recent,
            ListRef::User(id) => K::Playlist { id: id.clone() },
            ListRef::Album { title, artist, tracks } => K::Album { title: title.clone(), artist: artist.clone(), tracks: tracks.clone() },
            ListRef::ArtistLiked(a) => K::ArtistLiked { artist: a.clone() },
            ListRef::Downloads => K::Downloads,
        }
    }

    pub fn from_key(k: &glyph_core::ListKey) -> Self {
        use glyph_core::ListKey as K;
        match k {
            K::Liked => ListRef::Liked,
            K::Recent => ListRef::Recent,
            K::Playlist { id } => ListRef::User(id.clone()),
            K::Album { title, artist, tracks } => ListRef::Album { title: title.clone(), artist: artist.clone(), tracks: tracks.clone() },
            K::ArtistLiked { artist } => ListRef::ArtistLiked(artist.clone()),
            K::Downloads => ListRef::Downloads,
        }
    }

    /// Same list (albums by title + artist).
    pub fn same(&self, other: &ListRef) -> bool {
        self.key().same(&other.key())
    }
}

/// One step of the back history.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub view: View,
    pub list: Option<ListRef>,
    pub artist: Option<String>,
    pub profile: Option<String>,
}

/// Artist profile data (loaded once per session and name).
pub enum ArtistPhoto {
    Loading,
    Ready(PathBuf),
    Missing,
    RetryAt(Instant),
}

pub struct ArtistPage {
    pub name: String,
    pub tracks: Vec<Track>,
    pub loading: bool,
    pub info: Option<glyph_core::artist::ArtistInfo>,
    pub info_loading: bool,
    /// Fans + verified badge.
    pub stats: Option<glyph_core::catalog::ArtistStats>,
    /// Artists similar to this one (Last.fm, else Deezer); empty = none known.
    pub similar: Vec<String>,
    pub similar_loading: bool,
    /// The whole discography (catalog), newest first; empty while loading or
    /// unknown (the page then groups the songs found by album).
    pub releases: Vec<glyph_core::providers::deezer::Release>,
    pub show_all: bool,
    pub about_open: bool,
    _tasks: Vec<Task<()>>,
}

/// Sleep timer: playback pauses at a time or when the song ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sleep {
    /// Pause at `until`; `mins` is the picked length (marks it in the menu).
    At { until: Instant, mins: u32 },
    EndOfSong,
}

/// Right-click menus of the player controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlMenu {
    /// Sleep timer: lengths, end of song, off.
    Sleep,
    /// Sort order of the open list.
    Sort,
    /// Card or bar mini player.
    Mini,
    /// Off / all / one.
    Repeat,
    /// Off / smart / random.
    Shuffle,
    /// Previous button: jump back 10 / 30 s, restart, previous song.
    Back,
    /// Next button: jump ahead 10 / 30 s, next song.
    Forward,
    /// Volume: output device, mute, channel mode.
    Volume,
}

/// One entry under the sidebar navigation: a list or an artist page.
#[derive(Debug, Clone, PartialEq)]
pub enum SideItem {
    List(ListRef),
    Artist { name: String, track: Option<Track> },
}

impl SideItem {
    /// Id for hiding and for its use weight.
    pub fn id(&self) -> String {
        match self {
            SideItem::List(l) => glyph_core::Library::list_id(&l.key()),
            SideItem::Artist { name, .. } => glyph_core::Library::artist_id(name),
        }
    }
}

/// Right-click menu for a song.
#[derive(Debug, Clone)]
pub struct CtxMenu {
    pub track: Track,
    pub pos: gpui::Point<gpui::Pixels>,
    /// The list and index the song was opened from (for "remove from …").
    pub from: Option<(ListRef, usize)>,
    /// Showing the "add to playlist" flyout.
    pub playlists: bool,
    /// Which flyout `playlists` shows (`ctx_menu::SUB_*`).
    pub sub: u8,
    /// Menu for an artist (artist page "more" button) instead of a song.
    pub artist: Option<String>,
    /// Menu for a whole list (album, playlist) instead of one song;
    /// `track` is then its first song.
    pub list: Option<ListRef>,
    /// Opened on a sidebar entry: offers to hide it there (its sidebar id).
    pub sidebar: Option<String>,
}

/// First artist of "A, B", "A & B", "A feat. B", "A x B", "A и B".
pub fn primary_artist(s: &str) -> String {
    glyph_core::dedupe::primary_artist(s)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchFilter {
    All,
    Tracks,
    Albums,
    Artists,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BackendState {
    Connecting,
    Ready(BackendMode),
    Offline(String),
}

#[derive(Default)]
pub struct SearchState {
    pub query: String,
    pub seq: u64,
    pub results: Vec<Track>,
    pub seen: HashSet<(ProviderId, String)>,
    pub errors: Vec<(ProviderId, String)>,
    pub loading: bool,
    pub filter: Option<SearchFilter>,
    pub error: Option<String>,
    task: Option<Task<()>>,
    /// Songs whose lyrics contain the query, with the matching line.
    pub lyric_hits: Vec<(Track, String)>,
    lyric_task: Option<Task<()>>,
    /// Last result page asked for (0 = first) and whether more may come.
    pub page: usize,
    pub exhausted: bool,
    pub loading_more: bool,
    more_task: Option<Task<()>>,
}

#[derive(Default)]
pub struct LyricsState {
    pub key: Option<String>,
    pub lyrics: Option<Lyrics>,
    pub loading: bool,
    pub error: Option<String>,
    pub last_line: Option<usize>,
    /// Other versions of these lyrics (loaded on demand, best first).
    pub alts: Vec<Lyrics>,
    pub alts_loading: bool,
    /// The versions were looked up (also when none were found).
    pub alts_loaded: bool,
}

#[derive(Default)]
pub struct UpdateState {
    pub checking: bool,
    pub status: Option<UpdateStatus>,
    pub error: Option<String>,
    pub installing: Option<(u64, u64)>,
    pub installed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConnTest {
    Running,
    Ok(String),
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct CoverArt {
    pub key: String,
    pub path: Option<PathBuf>,
    pub ascii: Vec<(String, Option<Palette>)>,
}

/// Pages of the settings modal (Liner-style left navigation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPage {
    Playback,
    Audio,
    Appearance,
    Server,
    Account,
    /// Picture, banner and privacy (signed in only).
    Profile,
    Storage,
    /// Export, share and import settings.
    Transfer,
    /// Move a library over from another service.
    Migrate,
    /// Other apps on this computer (Discord Rich Presence).
    Integrations,
    About,
}

pub struct Glyph {
    pub focus: FocusHandle,
    pub view: View,
    pub open_list: Option<ListRef>,
    pub open_artist: Option<String>,
    /// Handle of the open user profile (`View::Profile`).
    pub open_profile: Option<String>,
    /// Home section shown on its own page: title, songs.
    pub section: Option<(String, Vec<Track>)>,
    /// Keyboard-focused song row index in the active list view.
    pub selected_row: Option<usize>,
    pub artists: std::collections::HashMap<String, ArtistPage>,
    pub artist_photos: std::collections::HashMap<String, ArtistPhoto>,
    pub history: Vec<Route>,
    pub ctx_menu: Option<CtxMenu>,
    /// Right-click menu of a player control: which, where, since when.
    pub control_menu: Option<(ControlMenu, gpui::Point<gpui::Pixels>, Instant)>,
    /// When the menu opened (fade/slide in) and a menu that is fading out.
    pub ctx_opened: Instant,
    pub ctx_closing: Option<(CtxMenu, Instant)>,
    /// Artist hero images rendered as ASCII, per source image and raster width
    /// (None = computing), plus replaced ones waiting to leave the GPU atlas.
    pub heroes: std::collections::HashMap<(PathBuf, bool, u32, u32), Option<Arc<gpui::RenderImage>>>,
    pub stale_heroes: Vec<Arc<gpui::RenderImage>>,
    /// Scroll state of the main view, the settings page, the queue and the lyrics
    /// (edge fades show only where there is more to scroll).
    pub main_scroll: gpui::ScrollHandle,
    pub settings_scroll: gpui::ScrollHandle,
    pub queue_scroll: gpui::ScrollHandle,
    pub lyrics_scroll: gpui::ScrollHandle,
    /// The user scrolled the lyrics by hand (auto-follow pauses a moment).
    pub lyrics_user_scroll: Option<Instant>,
    /// Last reported position and when it arrived (smooth position between reports).
    pos_anchor: Option<(Duration, Instant)>,
    pub name_dialog: Option<NameDialog>,
    /// "Add songs" panel (shares the queue's slot on the right).
    pub add_songs: Option<AddSongs>,
    pub add_input: Entity<TextInput>,
    pub add_scroll: gpui::ScrollHandle,
    /// Sidebar icon under the mouse and since when (hover animation).
    pub nav_hover: Option<(View, Instant)>,
    /// Fills the open album with its other songs (search in the background).
    album_task: Option<Task<()>>,
    /// Recently played keys last put into the taskbar / dock menu.
    dock_keys: Vec<String>,
    /// Set while playback starts from an artist's page: the artist is logged
    /// to "recently played" (instead of only the song) by `start_track`.
    pending_recent_artist: Option<(String, Option<Track>)>,
    /// Songs being downloaded right now (`track_key`).
    pub downloading: HashSet<String>,
    /// When songs finished downloading (`track_key`), for the check animation.
    pub download_done: std::collections::HashMap<String, Instant>,
    /// Audio outputs found on this computer (settings list).
    pub output_devices: Vec<String>,
    /// Album year + type per "title|artist" (None while loading / unknown).
    pub album_meta: std::collections::HashMap<String, Option<glyph_core::catalog::AlbumMeta>>,
    /// Home: songs picked for the user, and what is popular right now.
    pub recs: Vec<Track>,
    pub trending: Vec<Track>,
    feeds_loading: bool,
    /// Home: recent releases of followed artists (newest first).
    pub new_releases: Vec<glyph_core::providers::deezer::Release>,
    new_releases_loading: bool,
    /// Startup animation while it runs.
    pub splash: Option<crate::ui::splash::Splash>,
    /// Icon the mouse just left: (view, when, hover seconds at leave) for the ease-back.
    pub nav_leave: Option<(View, Instant, f32)>,
    /// Lyrics options open (since when, for the grow animation).
    pub lyrics_info: Option<Instant>,
    /// Sidebar list tile under the mouse (tooltip).
    pub side_hover: Option<(usize, Instant)>,
    pub playlist_name: Entity<TextInput>,
    /// "Paste a link" field of the New playlist dialog.
    pub playlist_link: Entity<TextInput>,
    /// Link / file import started from the dialog or the search bar.
    pub link_import: Option<LinkImport>,
    link_import_seq: u64,
    pub glow: Entity<GlowView>,
    pub search_input: Entity<TextInput>,
    pub server_input: Entity<TextInput>,
    pub server_bearer_input: Entity<TextInput>,
    /// Settings > Integrations: Discord application ID.
    pub discord_id_input: Entity<TextInput>,
    pub server_auth_required: bool,
    server_probe_seq: u64,
    /// Last official-server outage announced (see `glyph_core::outage`).
    outage_seen: u64,
    pub key_inputs: Vec<(&'static str, Entity<TextInput>)>,

    pub config: Config,
    pub backend: Option<Arc<Backend>>,
    /// Settings -> Account state and the sync loop.
    pub account: crate::ui::account::AccountUi,
    pub shared_preview: Option<(String, String, Option<String>)>,
    pending_share_link: Option<String>,
    /// User profiles (open page, own settings).
    pub profile: crate::ui::profile::ProfileUi,
    /// Settings export / import (Settings -> Backup & share).
    pub transfer: crate::ui::transfer::TransferUi,
    /// Settings -> Transfer music (library move from another service).
    pub migrate: crate::ui::migrate::MigrateUi,
    pub backend_state: BackendState,
    pub player: Option<Arc<PlayerController>>,
    pub player_ok: bool,
    pub player_error: Option<String>,
    pub player_state: Option<PlayerState>,
    player_events: Option<flume::Receiver<PlayerEvent>>,
    pub media_keys: Option<MediaKeys>,
    media_events: Option<flume::Receiver<MediaKeyEvent>>,
    hwnd: Option<usize>,
    /// Discord Rich Presence worker (Settings > Integrations).
    discord: Option<crate::discord::DiscordPresence>,
    /// Client ID the running worker was started with.
    discord_id: Option<String>,
    /// Last (track, playing, position/15 s) pushed to Discord.
    discord_sent: Option<(Option<(glyph_protocol::ProviderId, String)>, bool, u64)>,

    pub queue: Queue,
    pub queue_ok: bool,
    pub library: Library,
    pub library_ok: bool,

    pub current: Option<Track>,
    activity_recorded: bool,
    home_activity_cache: Option<(u64, Vec<Track>, Vec<String>)>,
    pub current_source: Option<ProviderId>,
    pub current_preview: bool,
    /// Track whose expired preview URL was already retried once (see
    /// [`App::retry_expired_preview`]); reset whenever playback starts.
    pub retried_expired: Option<Track>,
    /// Track whose broken local file was already dropped once (see
    /// [`App::repair_broken_file`]); reset whenever playback starts.
    pub repaired_file: Option<String>,
    /// Songs downloaded again after their file broke: if the new copy breaks
    /// too, it is only removed, not fetched a third time.
    pub redownloaded_broken: HashSet<String>,
    pub resolving: bool,
    /// Play/pause pressed while the new song was still being found: it is
    /// loaded paused instead of starting (the old song never resumes).
    pub hold_play: bool,
    /// Running sleep timer.
    pub sleep: Option<Sleep>,
    /// Next song handed to the player for a gapless start:
    /// (queue `track_key`, resolved track the player knows it as).
    preloaded: Option<(String, Track)>,
    preload_attempt: Option<(String, String, Instant)>,
    download_check_at: Instant,
    /// Volume to go back to when unmuting from the speaker icon.
    unmute_volume: f32,
    pub palette: Option<Palette>,
    pub cover: Option<CoverArt>,

    pub search: SearchState,
    pub lyrics: LyricsState,
    pub update: UpdateState,
    pub conn_test: Option<ConnTest>,
    /// yt-dlp status / install state for the settings page.
    pub ytdlp: crate::ui::ytdlp::YtDlpUi,

    pub show_queue: bool,
    /// "Connect to a device" panel (queue slot): where music plays.
    pub show_devices: bool,
    /// The Connect panel shows the jam (header button) instead of the devices.
    pub devices_jam: bool,
    pub devices_scroll: gpui::ScrollHandle,
    pub show_lyrics: bool,
    /// Now Playing shows only the lyrics (global player bar stays visible).
    pub lyrics_only: bool,
    /// Slide state of the player bar.
    pub bar_slide: Slide,
    /// Queue panel sliding in from the right.
    pub queue_slide: Slide,
    /// A-B loop of the current song (Alt+1 / Alt+2): start, end.
    pub loop_ab: (Option<Duration>, Option<Duration>),
    /// The song the loop belongs to; another song ends it.
    loop_song: Option<String>,
    /// The A-B loop the native player was last told about.
    loop_sent: Option<(Duration, Duration)>,
    /// Queue reorder in progress: (dragged index, slot it would land in).
    pub queue_drop: Option<(usize, usize)>,
    /// Song of an own playlist being dragged: (from, slot it would land in).
    pub list_drop: Option<(usize, usize)>,
    pub radio: bool,
    /// Does radio really work? `None` = not checked yet (switch stays usable).
    /// The server's `/v1/similar` needs a working Last.fm key; without one the
    /// switch is greyed out and explains itself on hover.
    pub radio_ok: Option<bool>,
    pub mini_on_top: bool,
    /// The open mini player window, if any.
    pub mini_window: Option<gpui::AnyWindowHandle>,
    /// The mini player is on screen (its window may stay around hidden).
    pub mini_shown: bool,
    /// Mini player as the big card instead of the slim bar (remembered).
    pub mini_big: bool,
    /// Card width of the mini player (resizable).
    pub mini_card_w: f32,
    pub mini_card_h: f32,
    /// Opening a list without counting it as use (Ctrl+number).
    nav_quiet: bool,
    /// The list the queue was started from (for list activity).
    pub(crate) playing_from: Option<ListRef>,
    /// Name of what the queue plays from ("Next from: …"): a list, an
    /// artist, a search, radio.
    pub queue_from: Option<String>,
    /// Playback started from this artist's page: their songs are not logged
    /// one by one in "recently played" (the artist entry stands for them).
    recent_artist_ctx: Option<String>,
    /// Range shown on the statistics page.
    pub stats_range: glyph_core::stats::Range,
    /// The statistics page shows the year recap of this year (`"2026"`).
    pub recap_year: Option<String>,
    /// Listening time not yet added to the statistics: (song, ms).
    stats_pending: Option<(Track, u64)>,
    stats_tick: Option<Instant>,
    stats_flushed: Instant,
    stats_synced: Instant,
    /// Verified badge per artist (lowercase name): `None` while it is looked up.
    pub verified: std::collections::HashMap<String, Option<bool>>,
    verified_inflight: usize,
    /// MPD output (Settings > Playback): songs play there, Glyph is the remote.
    pub mpd: Option<crate::mpd::Output>,
    pub mpd_input: Entity<TextInput>,
    /// MPD (or the network device) was playing on the last poll (its song
    /// ending moves the queue on).
    mpd_was_playing: bool,
    /// Search for playback devices on the network (Settings > Playback).
    pub discovery: Option<crate::devices::Discovery>,
    discovery_gen: u64,
    /// Playing on a network device (Cast, UPnP/DLNA, Sonos); Glyph is the remote.
    pub device_out: Option<crate::devices::Output>,
    /// The device could not play the current song (told once).
    device_failed: bool,
    /// The chosen remote output cannot play this song: it plays here.
    local_fallback: bool,
    /// The song last handed to an output, so it can move between outputs.
    last_load: Option<(Track, glyph_protocol::PlaybackSource)>,
    /// MCP server for AI assistants (Settings > Integrations).
    pub mcp: crate::mcp_tools::McpState,
    /// Jam: listening together (Connect panel).
    pub jam: JamUi,
    mcp_checked: Instant,
    /// Last redraw for the playing-song equalizer.
    eq_redraw: Instant,
    /// Window title last set (also updated while the window is in the background).
    window_title: String,
    window_handle: gpui::AnyWindowHandle,
    /// The saved config was applied (player prefs may be written back).
    config_loaded: bool,
    /// Unsaved config change: (last change, show the "saved" toast).
    config_dirty: Option<(Instant, bool)>,
    /// Unsaved local listening/navigation activity change (debounced).
    activity_dirty: Option<Instant>,
    library_save: Option<Task<()>>,
    /// Settings modal.
    pub settings_open: bool,
    /// Phone layout (`ui::compact`): settings show a category list first;
    /// true once a category page is open.
    pub settings_in_page: bool,
    /// Phone layout: the profile menu sliding in from the left.
    pub drawer_open: bool,
    pub drawer_slide: Slide,
    /// Phone library filter: 0 all, 1 playlists, 2 albums, 3 artists.
    pub lib_filter: u8,
    /// When settings opened / started closing (fade + lift).
    settings_at: Instant,
    settings_closing: Option<Instant>,
    pub settings_page: SettingsPage,
    pub settings_search: Entity<TextInput>,
    /// Which dropdown in the settings modal is open.
    pub open_select: Option<&'static str>,
    /// Selected sub-tab of the settings page (see `ui::settings::subtabs`).
    pub settings_sub: u8,
    pub settings_collapsed: HashSet<usize>,
    pub settings_group_motion: std::collections::HashMap<usize, (Instant, f32, f32)>,
    /// Storage breakdown for "Cache & Storage" (None = not measured yet).
    pub storage: Option<glyph_core::storage::StorageStats>,
    /// Which storage kinds "Clear cache" removes (order of StorageKind::ALL).
    pub storage_selected: [bool; 5],
    /// Notification stack, newest first.
    pub toasts: Vec<Toast>,
    toast_seq: u64,
    /// Toast being swiped and the mouse y where the swipe started.
    pub toast_drag: Option<(u64, f32)>,
    /// Mouse over the toast stack (fans it out, keeps it on screen).
    pub toast_hover: bool,
    /// Since when (long texts start scrolling after a moment).
    pub toast_hover_at: Option<Instant>,
    pub lang: crate::i18n::Lang,
    /// Eased text ink per background zone (0 light .. 1 dark).
    pub ink: [f32; 3],
    /// Cover cache: track key -> resolved local path (None = no cover / loading).
    pub covers: std::collections::HashMap<String, Option<PathBuf>>,
    /// Covers currently being fetched (drawn as skeletons).
    pub covers_loading: std::collections::HashSet<String>,
    /// Covers rendered as ASCII, per (track, cols, rows); `None` = computing.
    pub ascii_covers: std::collections::HashMap<(String, u32), Option<Arc<gpui::RenderImage>>>,
    pub embed: crate::embed::EmbedHost,
    /// Last (track, playing, position/5 s) pushed to the OS media controls.
    media_keys_sent: Option<(Option<(glyph_protocol::ProviderId, String)>, bool, u64)>,
    /// Re-render every tick while something animates (spinners).
    pub started: Instant,
    _subs: Vec<Subscription>,
    _poll: Task<()>,
    /// Writes a still pending config change when the app quits.
    _save_on_quit: gpui::Subscription,
}

/// A 0..1 slide-in/out value eased over time (player bar).
pub struct Slide {
    target: bool,
    from: f32,
    at: Instant,
}

impl Default for Slide {
    fn default() -> Self {
        Self { target: false, from: 0.0, at: Instant::now() - Duration::from_secs(10) }
    }
}

impl Slide {
    const DURATION: f32 = 0.32;

    fn raw(&self) -> f32 {
        let t = (self.at.elapsed().as_secs_f32() / Self::DURATION).min(1.0);
        // ease-out cubic
        let e = 1.0 - (1.0 - t).powi(3);
        let to = if self.target { 1.0 } else { 0.0 };
        self.from + (to - self.from) * e
    }

    /// Sets the target and returns the current value.
    pub fn update(&mut self, target: bool) -> f32 {
        if target != self.target {
            self.from = self.raw();
            self.target = target;
            self.at = Instant::now();
        }
        self.raw()
    }

    pub fn running(&self) -> bool {
        self.at.elapsed().as_secs_f32() < Self::DURATION
    }
}

/// "Popular" and "Recommended for you" as last loaded (data saver).
fn feeds_cache_path(scope: &PathBuf) -> PathBuf {
    scope.with_extension("feeds.json")
}

fn save_feeds_cache(scope: &PathBuf, trending: &[Track], recs: &[Track]) {
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if let Ok(bytes) = serde_json::to_vec(&(at, trending, recs)) {
        let _ = glyph_core::paths::write_atomic(&feeds_cache_path(scope), &bytes);
    }
}

/// The cached feeds when younger than `max_age_s` and not empty.
fn load_feeds_cache(scope: &PathBuf, max_age_s: u64) -> Option<(Vec<Track>, Vec<Track>)> {
    let (at, trending, recs): (u64, Vec<Track>, Vec<Track>) = serde_json::from_slice(&std::fs::read(feeds_cache_path(scope)).ok()?).ok()?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    (now.saturating_sub(at) < max_age_s && !(trending.is_empty() && recs.is_empty())).then_some((trending, recs))
}

/// "New from artists you follow" as last loaded, with the follow list it
/// was loaded for.
fn releases_cache_path(scope: &PathBuf) -> PathBuf {
    // v2: entries are checked against the album's main artist.
    scope.with_extension("releases-v2.json")
}

fn save_releases_cache(scope: &PathBuf, artists: &[String], releases: &[glyph_core::providers::deezer::Release]) {
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if let Ok(bytes) = serde_json::to_vec(&(at, artists, releases)) {
        let _ = glyph_core::paths::write_atomic(&releases_cache_path(scope), &bytes);
    }
}

/// The cached releases when younger than `max_age_s` and loaded for the same artists.
fn load_releases_cache(scope: &PathBuf, artists: &[String], max_age_s: u64) -> Option<Vec<glyph_core::providers::deezer::Release>> {
    type Cached = (u64, Vec<String>, Vec<glyph_core::providers::deezer::Release>);
    let (at, names, releases): Cached = serde_json::from_slice(&std::fs::read(releases_cache_path(scope)).ok()?).ok()?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    (now.saturating_sub(at) < max_age_s && names == artists).then_some(releases)
}

/// The song is (co-)credited to `name`.
fn by_artist(name: &str, credit: &str) -> bool {
    glyph_core::dedupe::by_artist(name, credit)
}

/// How long a toast stays.
pub const TOAST_FOR: Duration = Duration::from_millis(3500);
/// At most this many toasts are stacked; older ones drop off.
const MAX_TOASTS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToastIcon {
    Check,
    Info,
}

/// One notification of the top stack (`crate::ui::toasts`).
#[derive(Debug, Clone)]
pub struct Toast {
    pub id: u64,
    pub title: String,
    /// Detail after the dot, shown in caps (song, artist, playlist).
    pub sub: Option<String>,
    pub icon: ToastIcon,
    pub at: Instant,
    /// Current and wanted top offset (NaN until first drawn).
    pub y: f32,
    pub target: f32,
    /// Swipe offset (negative = up).
    pub drag: f32,
    /// Place in the collapsed deck (0 = front), eased.
    pub depth: f32,
    pub depth_target: f32,
    /// Swiped away: dismiss animation since.
    pub gone: Option<Instant>,
}

pub fn eq_gains(v: &[f32]) -> [f32; 10] {
    let mut g = [0.0; 10];
    for (i, x) in v.iter().take(10).enumerate() {
        g[i] = *x;
    }
    g
}

fn channels_of(m: glyph_core::config::ChannelMode) -> glyph_audio::spatial::Mode {
    use glyph_core::config::ChannelMode as C;
    match m {
        C::Mono => glyph_audio::spatial::Mode::Mono,
        C::Stereo => glyph_audio::spatial::Mode::Stereo,
        C::Spatial => glyph_audio::spatial::Mode::Spatial,
    }
}

fn level_of(l: glyph_core::config::VolumeLevel) -> glyph_audio::loudness::Level {
    use glyph_core::config::VolumeLevel as V;
    match l {
        V::Loud => glyph_audio::loudness::Level::Loud,
        V::Normal => glyph_audio::loudness::Level::Normal,
        V::Quiet => glyph_audio::loudness::Level::Quiet,
    }
}

pub fn track_key(t: &Track) -> String {
    format!("{:?}:{}", t.source, t.id)
}

fn raw_handle(window: &Window) -> Option<raw_window_handle::RawWindowHandle> {
    use raw_window_handle::HasWindowHandle;
    HasWindowHandle::window_handle(window).ok().map(|h| h.as_raw())
}

/// Raw main-window handle for souvlaki on Windows.
pub(crate) fn window_hwnd(window: &Window) -> Option<usize> {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(h) = HasWindowHandle::window_handle(window) {
            if let RawWindowHandle::Win32(w) = h.as_raw() {
                return Some(w.hwnd.get() as usize);
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        None
    }
}
