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
    flume_like, Backend, BackendMode, Config, Library, Lyrics, Queue, RepeatMode, UpdateInfo,
    UpdateStatus,
};
use glyph_protocol::{ProviderId, SearchEvent, Track};
use gpui::{App, AppContext, Context, Entity, FocusHandle, Subscription, Task, Window};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Quiet time after the last settings change before the file is written.
const CONFIG_SAVE_DELAY: Duration = Duration::from_millis(700);

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

/// "New playlist" / "Rename" dialog.
#[derive(Debug, Clone)]
pub struct NameDialog {
    /// Songs to add to the new playlist.
    pub tracks: Vec<Track>,
    /// Playlist id when renaming.
    pub rename: Option<String>,
    /// Naming an equalizer preset instead of a playlist.
    pub eq_preset: bool,
}

impl ListRef {
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
}

/// Artist profile data (loaded once per session and name).
pub struct ArtistPage {
    pub name: String,
    pub tracks: Vec<Track>,
    pub loading: bool,
    pub info: Option<glyph_core::artist::ArtistInfo>,
    pub info_loading: bool,
    /// Fans + verified badge.
    pub stats: Option<glyph_core::catalog::ArtistStats>,
    pub show_all: bool,
    pub about_open: bool,
    _tasks: Vec<Task<()>>,
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
    /// Menu for an artist (artist page "more" button) instead of a song.
    pub artist: Option<String>,
    /// Menu for a whole list (album, playlist) instead of one song;
    /// `track` is then its first song.
    pub list: Option<ListRef>,
}

/// First artist of "A, B", "A & B", "A feat. B", "A x B".
pub fn primary_artist(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut cut = s.len();
    for sep in [",", " & ", " feat", " ft.", " x ", " и ", " / "] {
        if let Some(i) = lower.find(sep) {
            cut = cut.min(i);
        }
    }
    s[..cut].trim().to_string()
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
}

#[derive(Default)]
pub struct LyricsState {
    pub key: Option<String>,
    pub lyrics: Option<Lyrics>,
    pub loading: bool,
    pub error: Option<String>,
    pub last_line: Option<usize>,
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
    Storage,
    /// Export, share and import settings.
    Transfer,
    About,
}

pub struct Glyph {
    pub focus: FocusHandle,
    pub view: View,
    pub open_list: Option<ListRef>,
    pub open_artist: Option<String>,
    pub artists: std::collections::HashMap<String, ArtistPage>,
    pub history: Vec<Route>,
    pub ctx_menu: Option<CtxMenu>,
    /// When the menu opened (fade/slide in) and a menu that is fading out.
    pub ctx_opened: Instant,
    pub ctx_closing: Option<(CtxMenu, Instant)>,
    /// Artist hero images rendered as ASCII, per source image (None = computing).
    pub heroes: std::collections::HashMap<(PathBuf, bool), Option<Arc<gpui::RenderImage>>>,
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
    /// Sidebar icon under the mouse and since when (hover animation).
    pub nav_hover: Option<(View, Instant)>,
    /// Fills the open album with its other songs (search in the background).
    album_task: Option<Task<()>>,
    /// Recently played keys last put into the taskbar / dock menu.
    dock_keys: Vec<String>,
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
    /// Startup animation while it runs.
    pub splash: Option<crate::ui::splash::Splash>,
    /// Icon the mouse just left: (view, when, hover seconds at leave) for the ease-back.
    pub nav_leave: Option<(View, Instant, f32)>,
    /// Lyrics options open (since when, for the grow animation).
    pub lyrics_info: Option<Instant>,
    /// Sidebar list tile under the mouse (tooltip).
    pub side_hover: Option<(usize, Instant)>,
    pub playlist_name: Entity<TextInput>,
    pub glow: Entity<GlowView>,
    pub search_input: Entity<TextInput>,
    pub server_input: Entity<TextInput>,
    pub key_inputs: Vec<(&'static str, Entity<TextInput>)>,

    pub config: Config,
    pub backend: Option<Arc<Backend>>,
    /// Settings -> Account state and the sync loop.
    pub account: crate::ui::account::AccountUi,
    /// Settings export / import (Settings -> Backup & share).
    pub transfer: crate::ui::transfer::TransferUi,
    pub backend_state: BackendState,
    pub player: Option<Arc<PlayerController>>,
    pub player_ok: bool,
    pub player_error: Option<String>,
    pub player_state: Option<PlayerState>,
    player_events: Option<flume::Receiver<PlayerEvent>>,
    pub media_keys: Option<MediaKeys>,
    media_events: Option<flume::Receiver<MediaKeyEvent>>,
    hwnd: Option<usize>,

    pub queue: Queue,
    pub queue_ok: bool,
    pub library: Library,
    pub library_ok: bool,

    pub current: Option<Track>,
    pub current_source: Option<ProviderId>,
    pub current_preview: bool,
    /// Track whose expired preview URL was already retried once (see
    /// [`App::retry_expired_preview`]); reset whenever playback starts.
    pub retried_expired: Option<Track>,
    pub resolving: bool,
    /// Next song handed to the player for a gapless start:
    /// (queue `track_key`, resolved track the player knows it as).
    preloaded: Option<(String, Track)>,
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
    pub show_lyrics: bool,
    /// Now Playing shows only the lyrics (global player bar stays visible).
    pub lyrics_only: bool,
    /// Slide state of the player bar.
    pub bar_slide: Slide,
    pub radio: bool,
    pub mini_on_top: bool,
    /// The saved config was applied (player prefs may be written back).
    config_loaded: bool,
    /// Unsaved config change: (last change, show the "saved" toast).
    config_dirty: Option<(Instant, bool)>,
    /// Settings modal.
    pub settings_open: bool,
    /// When settings opened / started closing (fade + lift).
    settings_at: Instant,
    settings_closing: Option<Instant>,
    pub settings_page: SettingsPage,
    pub settings_search: Entity<TextInput>,
    /// Which dropdown in the settings modal is open.
    pub open_select: Option<&'static str>,
    /// Selected sub-tab of the settings page (see `ui::settings::subtabs`).
    pub settings_sub: u8,
    /// Storage breakdown for "Cache & Storage" (None = not measured yet).
    pub storage: Option<glyph_core::storage::StorageStats>,
    /// Which storage kinds "Clear cache" removes (order of StorageKind::ALL).
    pub storage_selected: [bool; 4],
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

/// The song is (co-)credited to `name`.
fn by_artist(name: &str, credit: &str) -> bool {
    let lower = credit.to_lowercase();
    let mut parts = vec![lower.as_str()];
    for sep in [",", " & ", " feat. ", " feat ", " ft. ", " x ", " / "] {
        parts = parts.iter().flat_map(|p| p.split(sep)).collect();
    }
    parts.iter().any(|p| glyph_core::dedupe::artist_similarity(name, p.trim()) >= 0.85)
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

impl Glyph {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let glow = cx.new(|cx| GlowView::new(window, cx));
        let search_input =
            cx.new(|cx| TextInput::new("search.placeholder", cx));
        let server_input = cx.new(|cx| TextInput::new("settings.server.placeholder", cx));
        let settings_search = cx.new(|cx| TextInput::new("settings.search", cx));
        let playlist_name = cx.new(|cx| TextInput::new("dialog.playlist.placeholder", cx));
        let key_inputs: Vec<(&'static str, Entity<TextInput>)> = [
            "youtube",
            "jamendo_client_id",
            "soundcloud_client_id",
            "spotify_client_id",
            "spotify_client_secret",
            "lastfm",
        ]
        .into_iter()
        .map(|k| (k, cx.new(|cx| TextInput::new("settings.keys.unset", cx).masked())))
        .collect();

        let mut subs = vec![cx.subscribe(&search_input, |this, _, ev: &InputEvent, cx| {
            match ev {
                InputEvent::Changed(q) => this.on_query(q.clone(), false, cx),
                InputEvent::Submit(q) => this.on_query(q.clone(), true, cx),
                InputEvent::Cancel => {}
            }
        })];
        subs.push(cx.subscribe(&playlist_name, |this, _, ev: &InputEvent, cx| match ev {
            InputEvent::Submit(_) => this.submit_name_dialog(cx),
            InputEvent::Cancel => this.close_name_dialog(cx),
            InputEvent::Changed(_) => {}
        }));
        subs.push(cx.subscribe(&server_input, |this, _, ev: &InputEvent, cx| {
            if let InputEvent::Changed(v) | InputEvent::Submit(v) = ev {
                this.config.server_url = v.trim().to_string();
                this.conn_test = None;
                if matches!(ev, InputEvent::Submit(_)) {
                    this.save_config(cx);
                }
            }
        }));
        for (name, input) in &key_inputs {
            let name = *name;
            subs.push(cx.subscribe(input, move |this, _, ev: &InputEvent, cx| {
                if let InputEvent::Changed(v) | InputEvent::Submit(v) = ev {
                    let v = (!v.trim().is_empty()).then(|| v.trim().to_string());
                    let k = &mut this.config.keys;
                    match name {
                        "youtube" => k.youtube = v,
                        "jamendo_client_id" => k.jamendo_client_id = v,
                        "soundcloud_client_id" => k.soundcloud_client_id = v,
                        "spotify_client_id" => k.spotify_client_id = v,
                        "spotify_client_secret" => k.spotify_client_secret = v,
                        _ => k.lastfm = v,
                    }
                    if matches!(ev, InputEvent::Submit(_)) {
                        this.save_config(cx);
                    }
                }
            }));
        }

        let hwnd = window_hwnd(window);
        // Like / previous / play-pause / next under the taskbar preview.
        #[cfg(windows)]
        if let Some(h) = hwnd {
            crate::thumbbar::init(
                h,
                [t("ctx.like").to_string(), t("dock.previous").to_string(), t("dock.play_pause").to_string(), t("dock.next").to_string()],
            );
        }

        let poll = cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            if this.update(cx, |s, cx| s.poll(cx)).is_err() {
                break;
            }
        });

        let mut this = Self {
            focus: cx.focus_handle(),
            view: View::Home,
            open_list: None,
            open_artist: None,
            artists: Default::default(),
            history: Vec::new(),
            ctx_menu: None,
            ctx_opened: Instant::now(),
            ctx_closing: None,
            heroes: Default::default(),
            main_scroll: gpui::ScrollHandle::new(),
            settings_scroll: gpui::ScrollHandle::new(),
            queue_scroll: gpui::ScrollHandle::new(),
            lyrics_scroll: gpui::ScrollHandle::new(),
            lyrics_user_scroll: None,
            pos_anchor: None,
            name_dialog: None,
            nav_hover: None,
            album_task: None,
            // Read straight from disk: the async config load comes too late for the first frame.
            dock_keys: Vec::new(),
            downloading: HashSet::new(),
            download_done: Default::default(),
            album_meta: Default::default(),
            output_devices: Vec::new(),
            recs: Vec::new(),
            trending: Vec::new(),
            feeds_loading: false,
            splash: Config::load()
                .map(|c| c.visuals.startup_animation)
                .unwrap_or(true)
                .then(crate::ui::splash::Splash::new),
            nav_leave: None,
            lyrics_info: None,
            side_hover: None,
            playlist_name,
            glow,
            search_input,
            server_input,
            key_inputs,
            config: Config::default(),
            backend: None,
            backend_state: BackendState::Connecting,
            player: None,
            player_ok: false,
            player_error: None,
            player_state: None,
            player_events: None,
            media_keys: None,
            media_events: None,
            hwnd,
            queue: Queue::default(),
            queue_ok: true,
            library: Library::default(),
            library_ok: false,
            current: None,
            current_source: None,
            current_preview: false,
            retried_expired: None,
            resolving: false,
            preloaded: None,
            unmute_volume: 0.8,
            palette: None,
            cover: None,
            search: SearchState::default(),
            lyrics: LyricsState::default(),
            update: UpdateState::default(),
            conn_test: None,
            ytdlp: Default::default(),
            show_queue: false,
            show_lyrics: true,
            lyrics_only: false,
            bar_slide: Slide::default(),
            radio: true,
            mini_on_top: true,
            config_loaded: false,
            config_dirty: None,
            settings_open: false,
            settings_at: Instant::now(),
            settings_closing: None,
            settings_page: SettingsPage::Playback,
            account: Default::default(),
            transfer: Default::default(),
            settings_search,
            open_select: None,
            settings_sub: 0,
            storage: None,
            // Downloads are offline songs: opt-in, never cleared by default.
            storage_selected: [true, false, true, true],
            toasts: Vec::new(),
            toast_seq: 0,
            toast_drag: None,
            toast_hover: false,
            toast_hover_at: None,
            lang: crate::i18n::lang(),
            ink: [0.0; 3],
            covers: Default::default(),
            covers_loading: Default::default(),
            ascii_covers: Default::default(),
            embed: crate::embed::EmbedHost::new(raw_handle(window)),
            media_keys_sent: None,
            started: Instant::now(),
            _subs: subs,
            _poll: poll,
            _save_on_quit: cx.on_app_quit(|s: &mut Glyph, _| {
                let pending = s.config_dirty.take().is_some().then(|| s.config.clone());
                async move {
                    if let Some(cfg) = pending {
                        if let Err(e) = guard_res(|| cfg.save()) {
                            tracing::warn!("saving config on quit: {e}");
                        }
                    }
                }
            }),
        };
        this.boot(cx);
        this
    }

    // ------------------------------------------------------------ startup

    fn boot(&mut self, cx: &mut Context<Self>) {
        // Config → Backend, entirely off the UI thread.
        cx.spawn(async move |this, cx| {
            let config = cx
                .background_executor()
                .spawn(async { guard_res(Config::load) })
                .await;
            let config = match config {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("config: {e}");
                    Config::default()
                }
            };
            let cfg = config.clone();
            this.update(cx, |s, cx| s.apply_config(cfg, cx)).ok();
            let backend = cx
                .background_executor()
                .spawn(async move { guard_async(Backend::new(config)).await })
                .await;
            this.update(cx, |s, cx| s.backend_ready(backend, cx)).ok();
        })
        .detach();

        // Library + player, also in the background.
        cx.spawn(async move |this, cx| {
            let lib = cx
                .background_executor()
                .spawn(async { guard_res(Library::load) })
                .await;
            let player = cx
                .background_executor()
                .spawn(async { guard_res(PlayerController::new) })
                .await;
            this.update(cx, |s, cx| {
                match lib {
                    Ok(l) => {
                        s.library = l;
                        s.library_ok = true;
                    }
                    Err(e) => tracing::warn!("library: {e}"),
                }
                match player {
                    Ok(p) => {
                        let p = Arc::new(p);
                        p.set_crossfade(Duration::from_secs(s.config.audio.crossfade_s as u64));
                        p.set_loudness(level_of(s.config.audio.level), s.config.audio.normalize);
                        p.set_equalizer(s.config.audio.eq_enabled, eq_gains(&s.config.audio.eq_gains));
                        p.set_output_device(s.config.audio.output_device.clone());
                        p.set_volume(s.config.player.volume);
                        p.set_mini_player_options(MiniPlayerOptions { always_on_top: s.mini_on_top, position: None });
                        s.player_events = guard(|| p.events());
                        s.player_ok = true;
                        s.glow.update(cx, |g, _| g.set_player(Some(p.clone())));
                        // The embed webview is attached lazily in `poll` once an
                        // embed actually plays: a child webview makes Windows send
                        // synchronous messages to our window while GPUI holds its
                        // state ("RefCell already borrowed" on titlebar hover).
                        s.player = Some(p);
                    }
                    Err(e) => {
                        tracing::warn!("player: {e}");
                        s.player_error = Some(e);
                    }
                }
                s.init_media_keys();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn init_media_keys(&mut self) {
        let hwnd = self.hwnd.map(|h| h as *mut std::ffi::c_void);
        match guard_res(|| MediaKeys::new(hwnd)) {
            Ok(mk) => {
                self.media_events = guard(|| mk.events());
                self.media_keys = Some(mk);
            }
            Err(e) => tracing::warn!("media keys: {e}"),
        }
    }

    /// Volume, shuffle, repeat and panels as saved in `config.player`.
    fn restore_player_prefs(&mut self) {
        let prefs = self.config.player.clone();
        let repeat = match prefs.repeat.as_str() {
            "all" => RepeatMode::All,
            "one" => RepeatMode::One,
            _ => RepeatMode::Off,
        };
        guard(|| {
            self.queue.set_shuffle(prefs.shuffle);
            self.queue.set_repeat(repeat);
        });
        self.show_queue = prefs.show_queue;
        self.mini_on_top = prefs.mini_on_top;
        if let Some(p) = &self.player {
            p.set_volume(prefs.volume);
            p.set_mini_player_options(MiniPlayerOptions { always_on_top: prefs.mini_on_top, position: None });
        }
        // The cached state is read before the next refresh: keep it in step.
        if let Some(st) = self.player_state.as_mut() {
            st.volume = prefs.volume;
        }
        self.config_loaded = true;
    }

    /// Player prefs as they are now (`None` before the config is loaded).
    fn current_player_prefs(&self) -> Option<glyph_core::config::PlayerPrefs> {
        if !self.config_loaded {
            return None;
        }
        let volume = self.player_state.as_ref().map_or(self.config.player.volume, |s| s.volume);
        let repeat = match self.repeat() {
            RepeatMode::Off => "off",
            RepeatMode::All => "all",
            RepeatMode::One => "one",
        };
        Some(glyph_core::config::PlayerPrefs {
            volume,
            shuffle: self.shuffle(),
            repeat: repeat.into(),
            show_queue: self.show_queue,
            mini_on_top: self.mini_on_top,
        })
    }

    /// Notes changed player prefs (saved quietly by `flush_config`).
    fn save_player_prefs(&mut self, cx: &mut Context<Self>) {
        if let Some(prefs) = self.current_player_prefs() {
            if prefs != self.config.player {
                self.config.player = prefs;
                // Keeps a pending settings toast; restarts the delay.
                let toast = self.config_dirty.is_some_and(|(_, t)| t);
                self.config_dirty = Some((Instant::now(), toast));
            }
        }
        self.flush_config(cx);
    }

    pub(crate) fn apply_config(&mut self, config: Config, cx: &mut Context<Self>) {
        self.server_input
            .update(cx, |i, cx| i.set_value(config.server_url.clone(), cx));
        for (name, input) in &self.key_inputs {
            let k = &config.keys;
            let v = match *name {
                "youtube" => &k.youtube,
                "jamendo_client_id" => &k.jamendo_client_id,
                "soundcloud_client_id" => &k.soundcloud_client_id,
                "spotify_client_id" => &k.spotify_client_id,
                "spotify_client_secret" => &k.spotify_client_secret,
                _ => &k.lastfm,
            };
            let v = v.clone().unwrap_or_default();
            input.update(cx, |i, cx| i.set_value(v, cx));
        }
        let lang = crate::config_ext::language(&config);
        crate::i18n::set_lang(lang);
        self.lang = lang;
        let intensity = config.visuals.intensity;
        self.glow.update(cx, |g, _| g.set_intensity(intensity));
        self.config = config;
        self.restore_player_prefs();
        if let Some(p) = &self.player {
            p.set_crossfade(Duration::from_secs(self.config.audio.crossfade_s as u64));
            p.set_loudness(level_of(self.config.audio.level), self.config.audio.normalize);
            p.set_equalizer(self.config.audio.eq_enabled, eq_gains(&self.config.audio.eq_gains));
        }
        cx.notify();
    }

    fn backend_ready(&mut self, res: Result<Backend, String>, cx: &mut Context<Self>) {
        match res {
            Ok(b) => {
                let b = Arc::new(b);
                self.backend_state = match guard(|| b.mode()) {
                    Some(m) => BackendState::Ready(m),
                    None => BackendState::Ready(BackendMode::Direct),
                };
                self.backend = Some(b);
                self.load_feeds(cx);
                // Misses recorded without a backend are not real misses.
                self.covers.retain(|_, v| v.is_some());
                self.ascii_covers.clear();
                if self.config.updates.check_on_start {
                    self.check_updates(false, cx);
                }
                self.start_sync_loop(cx);
            }
            Err(e) => {
                tracing::warn!("backend offline: {e}");
                self.backend_state = BackendState::Offline(e);
            }
        }
        cx.notify();
    }

    pub fn retry_backend(&mut self, cx: &mut Context<Self>) {
        self.backend_state = BackendState::Connecting;
        let config = self.config.clone();
        cx.spawn(async move |this, cx| {
            let backend = cx
                .background_executor()
                .spawn(async move { guard_async(Backend::new(config)).await })
                .await;
            this.update(cx, |s, cx| s.backend_ready(backend, cx)).ok();
        })
        .detach();
        cx.notify();
    }

    // ------------------------------------------------------------ polling

    fn poll(&mut self, cx: &mut Context<Self>) {
        let mut dirty = false;
        self.save_player_prefs(cx);
        if let Some(p) = self.player.clone() {
            let embed_playing = self.embed_playing();
            if embed_playing {
                self.embed.attach(&p);
            }
            // Touch the webview only while an embed is involved, so normal
            // (native) playback never triggers child-window messages.
            if embed_playing || self.embed.is_visible() {
                let vis = self.view == View::NowPlaying && embed_playing;
                self.embed.set_visible(&p, vis);
                self.embed.pump(&p);
            }
        }

        if let (Some(p), true) = (self.player.clone(), self.player_ok) {
            match guard(|| p.state()) {
                Some(st) => {
                    if self.player_state.as_ref() != Some(&st) {
                        let playing = st.status == Status::Playing;
                        self.glow.update(cx, |g, _| g.set_playing(playing));
                        if let Some(mk) = &self.media_keys {
                            // The OS media controls only need track, play state
                            // and a coarse position; pushing every state change
                            // (~60/s) means needless OS calls on the UI thread.
                            let key = (
                                st.track.as_ref().map(|t| (t.source, t.id.clone())),
                                playing,
                                st.position.as_secs() / 5,
                            );
                            if self.media_keys_sent.as_ref() != Some(&key) {
                                let pos = st.position;
                                let track = st.track.clone();
                                guard(|| mk.set_now_playing(track.as_ref(), playing, pos));
                                self.media_keys_sent = Some(key);
                            }
                        }
                        if self.pos_anchor.map(|a| a.0) != Some(st.position) {
                            self.pos_anchor = Some((st.position, Instant::now()));
                        }
                        self.player_state = Some(st);
                        dirty = true;
                    }
                }
                None => self.player_ok = false,
            }
        }
        let events: Vec<PlayerEvent> = self
            .player_events
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for ev in events {
            dirty = true;
            match ev {
                PlayerEvent::Ended => self.next(true, cx),
                PlayerEvent::NearEnd => self.preload_next(cx),
                PlayerEvent::Error(e) => {
                    // Preview URLs are signed and expire after ~15 min. If the
                    // one we are holding has expired, resolve once more (which
                    // fetches a fresh URL) and replay instead of just failing.
                    if self.retry_expired_preview(&e, cx) {
                        tracing::info!("expired preview URL, re-resolving once");
                    } else {
                        self.toast_info(tf("toast.player_error", &[("e", &e)]));
                    }
                }
                PlayerEvent::StatusChanged(_) => {}
            }
        }
        let keys: Vec<MediaKeyEvent> = self
            .media_events
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for k in keys {
            dirty = true;
            match k {
                MediaKeyEvent::Play => self.with_player(|p| p.play()),
                MediaKeyEvent::Pause => self.with_player(|p| p.pause()),
                MediaKeyEvent::Toggle => self.toggle_play(cx),
                MediaKeyEvent::Next => self.next(false, cx),
                MediaKeyEvent::Previous => self.previous(cx),
                MediaKeyEvent::Stop => self.with_player(|p| p.stop()),
                MediaKeyEvent::SeekForward => self.seek_relative(5.0),
                MediaKeyEvent::SeekBackward => self.seek_relative(-5.0),
            }
        }

        // Lyric line changes pulse the glow.
        if let (Some(l), Some(st)) = (&self.lyrics.lyrics, &self.player_state) {
            let off = self.lyrics_offset();
            let pos = st.position.as_millis() as u64;
            let line = guard(|| l.active_line(pos, off)).flatten();
            if line != self.lyrics.last_line {
                self.lyrics.last_line = line;
                if line.is_some() {
                    self.glow.update(cx, |g, _| g.pulse());
                }
                dirty = true;
            }
        }

        // Adaptive text ink: ease towards the better-contrast colour (~300 ms).
        crate::theme::set_lum_grid(self.glow.read(cx).lum_grid());
        if crate::theme::update_inks() {
            dirty = true;
        }
        // Requests from another launch (jump list click, second start).
        #[cfg(windows)]
        {
            for id in crate::thumbbar::take_clicks() {
                match id {
                    crate::thumbbar::LIKE => {
                        if let Some(cur) = self.current.clone() {
                            self.toggle_like(&cur, cx);
                        }
                    }
                    crate::thumbbar::PREV => self.previous(cx),
                    crate::thumbbar::PLAY => self.toggle_play(cx),
                    crate::thumbbar::NEXT => self.next(false, cx),
                    _ => {}
                }
                dirty = true;
            }
            let liked = self.current.as_ref().is_some_and(|c| self.is_liked(c));
            crate::thumbbar::set_state(self.is_playing(), liked);
        }
        while let Some(req) = crate::instance::try_recv() {
            if let Some(n) = req.strip_prefix("dock ").and_then(|n| n.parse::<usize>().ok()) {
                self.dock_action(n, cx);
            }
            cx.activate(true);
        }
        self.update_dock_menu(cx);
        let scroll = -f32::from(self.main_scroll.offset().y);
        self.glow.update(cx, |g, cx| {
            if g.set_scroll(scroll) {
                cx.notify();
            }
        });
        let zones = self.glow.read(cx).zones();
        for (ink, lum) in self.ink.iter_mut().zip(zones) {
            let target = crate::theme::ink_for(lum);
            if (*ink - target).abs() > 0.001 {
                let step = 0.016 / 0.3;
                *ink = if target > *ink { (*ink + step).min(target) } else { (*ink - step).max(target) };
                dirty = true;
            }
        }
        if self.settings_closing.is_some_and(|at| at.elapsed() > Duration::from_millis(160)) {
            self.settings_closing = None;
            dirty = true;
        }
        let before = self.toasts.len();
        let held = self.toast_drag.map(|(id, _)| id);
        let hovered = self.toast_hover;
        self.toasts.retain(|t| match t.gone {
            Some(g) => g.elapsed().as_secs_f32() < crate::ui::toasts::GONE_S,
            None => hovered || held == Some(t.id) || t.at.elapsed() <= TOAST_FOR,
        });
        if self.toasts.len() != before {
            dirty = true;
        }
        if self.is_animating() || crate::ui::widgets::take_skeleton_seen() {
            dirty = true;
        }
        if dirty {
            cx.notify();
        }
    }

    pub fn is_animating(&self) -> bool {
        let toast_fading = crate::ui::toasts::animating(self);
        let settings_anim = (self.settings_open && self.settings_at.elapsed() < Duration::from_millis(220)) || self.settings_closing.is_some();
        let nav_anim = self.nav_hover.is_some_and(|(v, at)| v == View::NowPlaying || at.elapsed() < Duration::from_millis(700))
            || self.nav_leave.is_some_and(|(_, at, _)| at.elapsed().as_secs_f32() < crate::ui::chrome::NAV_EASE_BACK + 0.05);
        let menu_anim = (self.ctx_menu.is_some() && self.ctx_opened.elapsed() < Duration::from_millis(200)) || self.ctx_closing.is_some();
        let tip_anim = self.side_hover.is_some_and(|(_, at)| at.elapsed() < Duration::from_millis(600));
        let info_anim = self.lyrics_info.is_some_and(|at| at.elapsed() < Duration::from_millis(300));
        toast_fading
            || crate::ui::widgets::reveal_active()
            || info_anim
            || !self.downloading.is_empty()
            || self.download_done.values().any(|at| at.elapsed().as_secs_f32() < crate::ui::widgets::DL_DONE_ANIM_S)
            || settings_anim
            || crate::ui::settings::select_animating()
            || crate::ui::settings::flash_animating()
            || tip_anim
            || menu_anim
            || self.splash.is_some()
            || nav_anim
            || self.bar_slide.running()
            || self.search.loading
            || self.is_loading()
            || self.lyrics.loading
            || self.update.checking
            || self.update.installing.is_some()
            || matches!(self.conn_test, Some(ConnTest::Running))
            || self.backend_state == BackendState::Connecting
            || (self.view == View::NowPlaying && self.is_playing())
    }

    // ------------------------------------------------------------ helpers

    /// Confirmation toast. "Title · detail" is split at the first dot.
    pub fn toast(&mut self, msg: impl Into<String>) {
        self.push_toast(msg.into(), ToastIcon::Check);
    }

    /// Neutral / problem toast (info icon instead of the check).
    pub fn toast_info(&mut self, msg: impl Into<String>) {
        self.push_toast(msg.into(), ToastIcon::Info);
    }

    /// Toast with an explicit detail part.
    pub fn toast_sub(&mut self, title: impl Into<String>, sub: impl Into<String>) {
        self.push_toast(format!("{} · {}", title.into(), sub.into()), ToastIcon::Check);
    }

    fn push_toast(&mut self, msg: String, icon: ToastIcon) {
        tracing::info!("{msg}");
        let (title, sub) = match msg.split_once(" · ") {
            Some((a, b)) if !b.trim().is_empty() => (a.to_string(), Some(b.to_string())),
            _ => (msg, None),
        };
        let now = Instant::now();
        // The same message again (e.g. one error repeating): bring the
        // existing toast to the front instead of stacking copies.
        if let Some(i) = self.toasts.iter().position(|t| t.gone.is_none() && t.title == title && t.sub == sub) {
            let mut t = self.toasts.remove(i);
            t.at = now;
            self.toasts.insert(0, t);
            return;
        }
        self.toast_seq += 1;
        self.toasts.insert(
            0,
            Toast {
                id: self.toast_seq,
                title,
                sub,
                icon,
                at: now,
                y: f32::NAN,
                target: 0.0,
                drag: 0.0,
                depth: 0.0,
                depth_target: 0.0,
                gone: None,
            },
        );
        // Over the limit: the oldest visible ones swipe away.
        let mut live = 0;
        for t in self.toasts.iter_mut() {
            if t.gone.is_none() {
                live += 1;
                if live > MAX_TOASTS {
                    t.gone = Some(now);
                }
            }
        }
    }

    fn with_player(&mut self, f: impl FnOnce(&PlayerController)) {
        if let Some(p) = &self.player {
            if guard(|| f(p)).is_none() {
                self.toast_info(t("toast.player_not_ready"));
            }
        } else {
            self.toast_info(t("status.player_offline"));
        }
    }

    /// A song is being found (resolve, yt-dlp) or opened by the player.
    pub fn is_loading(&self) -> bool {
        self.resolving || self.player_state.as_ref().is_some_and(|s| s.status == Status::Loading)
    }

    pub fn is_playing(&self) -> bool {
        self.player_state.as_ref().is_some_and(|s| s.status == Status::Playing)
    }

    pub fn position(&self) -> Duration {
        self.player_state.as_ref().map(|s| s.position).unwrap_or_default()
    }

    /// Position moving smoothly between player reports (embedded players
    /// report only a few times a second); used for lyrics.
    pub fn display_position(&self) -> Duration {
        let base = self.position();
        match self.pos_anchor {
            Some((p, at)) if p == base && self.is_playing() => base + at.elapsed().min(Duration::from_millis(800)),
            _ => base,
        }
    }

    pub fn duration(&self) -> Option<Duration> {
        self.player_state
            .as_ref()
            .and_then(|s| s.duration)
            .or_else(|| self.current.as_ref()?.duration_ms.map(Duration::from_millis))
    }

    pub fn volume(&self) -> f32 {
        self.player_state.as_ref().map(|s| s.volume).unwrap_or(0.8)
    }

    pub fn is_liked(&self, t: &Track) -> bool {
        self.library_ok && guard(|| self.library.is_liked(t)).unwrap_or(false)
    }

    pub fn lyrics_offset(&self) -> i64 {
        match &self.current {
            Some(t) if self.library_ok => guard(|| self.library.lyrics_offset(t)).unwrap_or(0),
            _ => 0,
        }
    }

    pub fn upcoming(&self) -> Vec<Track> {
        guard(|| self.queue.upcoming().into_iter().cloned().collect()).unwrap_or_default()
    }

    pub fn shuffle(&self) -> bool {
        guard(|| self.queue.shuffle()).unwrap_or(false)
    }

    pub fn repeat(&self) -> RepeatMode {
        guard(|| self.queue.repeat()).unwrap_or_default()
    }

    pub fn spinner(&self) -> char {
        const S: [char; 4] = ['|', '/', '-', '\\'];
        S[(self.started.elapsed().as_millis() / 110 % 4) as usize]
    }

    fn save_library(&mut self, cx: &mut Context<Self>) {
        if !self.library_ok {
            return;
        }
        let lib = self.library.clone();
        cx.background_executor()
            .spawn(async move {
                if let Err(e) = guard_res(|| lib.save()) {
                    tracing::warn!("library save: {e}");
                }
            })
            .detach();
        self.schedule_sync(cx);
    }

    /// Saves the config once changes stop for a moment (a slider drag is
    /// one save, not one per step); see [`Glyph::flush_config`].
    pub fn save_config(&mut self, cx: &mut Context<Self>) {
        self.config_dirty = Some((Instant::now(), true));
        // Settings follow the account (debounced like library changes).
        self.schedule_sync(cx);
    }

    /// Settings that arrived from another device: apply and save them quietly.
    pub(crate) fn adopt_synced_config(&mut self, config: Config, providers_changed: bool, cx: &mut Context<Self>) {
        self.apply_config(config, cx);
        let toast = self.config_dirty.is_some_and(|(_, t)| t);
        self.config_dirty = Some((Instant::now(), toast));
        if providers_changed {
            // Sources and keys are read when the backend is built.
            self.retry_backend(cx);
        }
    }

    /// Writes a pending config change after [`CONFIG_SAVE_DELAY`] without
    /// further changes. `toast`: a settings change (not player prefs).
    fn flush_config(&mut self, cx: &mut Context<Self>) {
        let Some((at, toast)) = self.config_dirty else { return };
        if at.elapsed() < CONFIG_SAVE_DELAY {
            return;
        }
        self.config_dirty = None;
        let cfg = self.config.clone();
        if !toast {
            cx.background_executor()
                .spawn(async move {
                    if let Err(e) = guard_res(|| cfg.save()) {
                        tracing::warn!("saving player prefs: {e}");
                    }
                })
                .detach();
            return;
        }
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move { guard_res(|| cfg.save()) })
                .await;
            this.update(cx, |s, cx| {
                match r {
                    Ok(()) => s.toast_info(t("toast.config_saved")),
                    Err(e) => s.toast_info(tf("toast.config_error", &[("e", &e)])),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ------------------------------------------------------------ navigation

    pub fn navigate(&mut self, view: View, window: &mut Window, cx: &mut Context<Self>) {
        if view == View::Settings {
            self.open_settings(None, cx);
            return;
        }
        // Sidebar navigation starts a fresh history.
        self.main_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        self.history.clear();
        self.ctx_menu = None;
        self.view = view;
        if view == View::Search {
            let f = self.search_input.read(cx).focus.clone();
            window.focus(&f, cx);
        }
        if self.current.is_none() || view == View::Settings {
            let preset = match view {
                View::Home => Preset::Strahl,
                View::Search => Preset::Band,
                View::Library => Preset::Horizont,
                View::NowPlaying => Preset::Sonnenaufgang,
                View::Settings => Preset::Horizont,
                View::List => Preset::Horizont,
                View::Artist => Preset::Sonnenaufgang,
            };
            self.glow.update(cx, |g, _| g.set_preset(preset));
        }
        cx.notify();
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let f = self.search_input.read(cx).focus.clone();
        window.focus(&f, cx);
        cx.notify();
    }

    // ------------------------------------------------------------ search

    fn on_query(&mut self, q: String, immediate: bool, cx: &mut Context<Self>) {
        // A pasted settings code opens the import in settings.
        if glyph_core::settings_share::is_code(&q) {
            let code = q.clone();
            self.search_input.update(cx, |i, cx| i.set_value(String::new(), cx));
            self.import_settings_text(&code, cx);
            return;
        }
        // A pasted playlist code or link imports instead of searching.
        if glyph_core::share::is_code(&q) || glyph_core::share::link_id(&q).is_some() {
            let code = q.clone();
            self.search_input.update(cx, |i, cx| i.set_value(String::new(), cx));
            self.import_share_code(&code, cx);
            return;
        }
        if glyph_core::eq::is_code(&q) {
            let code = q.clone();
            self.search_input.update(cx, |i, cx| i.set_value(String::new(), cx));
            self.import_eq_code(&code, cx);
            return;
        }
        self.search.query = q.clone();
        self.search.seq += 1;
        let seq = self.search.seq;
        if self.view != View::Search && !q.trim().is_empty() {
            self.view = View::Search;
            self.main_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        }
        if q.trim().is_empty() {
            self.search.task = None;
            self.search.loading = false;
            self.search.results.clear();
            self.search.seen.clear();
            self.search.errors.clear();
            self.search.error = None;
            cx.notify();
            return;
        }
        self.search.loading = true;
        cx.notify();
        self.search.task = Some(cx.spawn(async move |this, cx| {
            if !immediate {
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
            }
            let stream = this
                .update(cx, |s, _| {
                    if s.search.seq != seq {
                        return Err(None);
                    }
                    s.search.results.clear();
                    s.search.seen.clear();
                    s.search.errors.clear();
                    s.search.error = None;
                    let Some(b) = s.backend.clone() else {
                        return Err(Some(t("status.backend_offline").to_string()));
                    };
                    let (plain, _) = glyph_core::taste::parse_query(q.trim());
                    guard(|| b.search(&plain)).ok_or(Some(t("error.not_ready").to_string()))
                })
                .ok();
            let mut stream = match stream {
                Some(Ok(s)) => s,
                Some(Err(Some(e))) => {
                    this.update(cx, |s, cx| {
                        s.search.loading = false;
                        s.search.error = Some(e);
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                _ => return,
            };
            use futures::{FutureExt, StreamExt};
            loop {
                let next = std::panic::AssertUnwindSafe(stream.next()).catch_unwind().await;
                let ev = match next {
                    Ok(Some(ev)) => ev,
                    Ok(None) => SearchEvent::Done,
                    Err(_) => SearchEvent::Done,
                };
                let done = matches!(ev, SearchEvent::Done);
                let alive = this
                    .update(cx, |s, cx| {
                        if s.search.seq != seq {
                            return false;
                        }
                        match ev {
                            SearchEvent::Results { tracks, .. } => {
                                let (plain, phrases) = glyph_core::taste::parse_query(&s.search.query);
                                let taste = s.taste();
                                for t in tracks {
                                    if s.is_hidden_artist(&t.artist) || !s.allowed(&t) || !glyph_core::taste::matches_phrases(&t, &phrases) {
                                        continue;
                                    }
                                    if s.search.seen.insert((t.source, t.id.clone())) {
                                        // Ranked insert: text match, odd versions, the user's taste.
                                        let sc = glyph_core::taste::score(&t, &plain, &taste);
                                        let at = s
                                            .search
                                            .results
                                            .iter()
                                            .position(|x| glyph_core::taste::score(x, &plain, &taste) < sc)
                                            .unwrap_or(s.search.results.len());
                                        s.search.results.insert(at, t);
                                    } else if let Some(x) = s.search.results.iter_mut().find(|x| x.source == t.source && x.id == t.id) {
                                        *x = t;
                                    }
                                }
                            }
                            SearchEvent::ProviderError { provider, message } => {
                                s.search.errors.push((provider, message))
                            }
                            SearchEvent::Done => s.search.loading = false,
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if done || !alive {
                    break;
                }
            }
        }));
    }

    // ------------------------------------------------------------ playback

    /// Artists in the current results, best matches for the query first:
    /// (name, a song of theirs for the picture).
    pub fn search_artists(&self, max: usize) -> Vec<(String, Track)> {
        let q = glyph_core::taste::parse_query(self.search.query.trim()).0.to_lowercase();
        let mut found: Vec<(String, Track, usize)> = Vec::new();
        for tr in &self.search.results {
            let name = primary_artist(&tr.artist);
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
        found.retain(|(n, _, c)| (score(n, 0) > 0 || *c >= 3) && !self.library.is_hidden(n));
        found.sort_by_key(|(n, _, c)| -score(n, *c));
        found.into_iter().take(max).map(|(n, t, _)| (n, t)).collect()
    }

    /// Query completions taken from the current results (titles and artists
    /// that contain what was typed), for the chips above the results.
    pub fn search_completions(&self, max: usize) -> Vec<String> {
        let q = glyph_core::taste::parse_query(self.search.query.trim()).0.to_lowercase();
        if q.len() < 2 {
            return Vec::new();
        }
        let mut out: Vec<String> = Vec::new();
        for tr in &self.search.results {
            for cand in [tr.title.as_str(), primary_artist(&tr.artist).as_str()] {
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

    /// Runs a search for `q` as if it had been typed.
    pub fn search_for(&mut self, q: String, cx: &mut Context<Self>) {
        self.search_input.update(cx, |i, cx| i.set_value(q.clone(), cx));
        self.on_query(q, true, cx);
    }

    /// Songs for the search dropdown: search history, else recently played.
    pub fn search_suggestions(&self) -> (bool, Vec<glyph_core::SearchItem>) {
        if !self.library.search_items.is_empty() {
            (true, self.library.search_items.iter().take(8).cloned().collect())
        } else {
            (false, self.library.recent.iter().take(8).map(|t| glyph_core::SearchItem::Song { track: t.clone() }).collect())
        }
    }

    /// Open an artist picked from search (remembered in recent searches).
    pub fn pick_search_artist(&mut self, name: &str, track: Option<Track>, window: &mut Window, cx: &mut Context<Self>) {
        if self.library_ok && guard(|| self.library.push_search_artist(name, track)).is_some() {
            self.save_library(cx);
        }
        let f = self.focus.clone();
        window.focus(&f, cx);
        self.open_artist(name, cx);
    }

    /// Play a song picked from the search dropdown and close it.
    pub fn play_suggestion(&mut self, track: Track, window: &mut Window, cx: &mut Context<Self>) {
        if self.library_ok {
            let picked = track.clone();
            guard(|| self.library.push_search(picked));
        }
        self.play_list(vec![track], 0, cx);
        let f = self.focus.clone();
        window.focus(&f, cx);
    }

    pub fn remove_search_history(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.library_ok && guard(|| self.library.remove_search(index)).is_some() {
            self.save_library(cx);
        }
        cx.notify();
    }

    /// Replace the queue with `tracks` and start at `index`.
    pub fn play_list(&mut self, tracks: Vec<Track>, index: usize, cx: &mut Context<Self>) {
        let Some(track) = tracks.get(index).cloned() else { return };
        // Picked from search results: remember it for the search dropdown
        // (saved together with "recent" in `play_track`).
        if self.library_ok && self.view == View::Search {
            let picked = track.clone();
            guard(|| self.library.push_search(picked));
        }
        if guard(|| self.queue.set(tracks, index)).is_none() {
            self.queue_ok = false;
        }
        self.play_track(track, cx);
    }

    pub fn play_track(&mut self, track: Track, cx: &mut Context<Self>) {
        self.start_track(track, None, cx);
    }

    /// `adopted`: the player already moved on to this song gaplessly (as the
    /// given resolved track), so only the UI follows; nothing is reloaded.
    fn start_track(&mut self, track: Track, adopted: Option<Track>, cx: &mut Context<Self>) {
        let key = track_key(&track);
        self.current = Some(track.clone());
        self.current_source = Some(adopted.as_ref().map_or(track.source, |a| a.source));
        self.current_preview = adopted.is_some() && self.player_state.as_ref().is_some_and(|s| s.is_preview);
        self.retried_expired = None;
        self.resolving = adopted.is_none();
        if adopted.is_none() {
            self.preloaded = None;
            // Fade the old song out right away instead of letting it run
            // until the new one has loaded.
            self.with_player(|p| p.pause());
        }
        if self.library_ok && guard(|| self.library.push_recent(track.clone())).is_some() {
            self.library.record_play(&track);
            self.save_library(cx);
        }
        let seed = format!("{}{}", track.artist, track.title);
        let palette = Palette::from_seed(&seed);
        self.palette = Some(palette);
        self.glow.update(cx, |g, _| {
            g.set_palette(Some(palette));
            g.set_preset(Preset::from_seed(&seed));
        });
        // The player bar and Now Playing cover "ASCII in" again with every new
        // song, also when that cover was already shown somewhere before.
        for size in [crate::ui::player_bar::COVER, crate::ui::now_playing::COVER] {
            crate::ui::widgets::reset_reveals(&format!("cover:{key}:{size}"));
        }
        self.cover = Some(CoverArt {
            key: key.clone(),
            path: None,
            ascii: ascii::pattern(&seed, 34, 17),
        });
        self.lyrics = LyricsState {
            key: Some(key.clone()),
            loading: true,
            ..Default::default()
        };
        cx.notify();

        let Some(backend) = self.backend.clone() else {
            self.resolving = false;
            self.lyrics.loading = false;
            self.toast_info(t("status.backend_offline"));
            return;
        };

        // Resolve → load → play.
        if adopted.is_some() {
            self.prefetch_upcoming();
        } else {
            let b = backend.clone();
            let t = track.clone();
            let k = key.clone();
            cx.spawn(async move |this, cx| {
                let r = guard_async(b.resolve(&t)).await;
                this.update(cx, |s, cx| {
                    if s.current.as_ref().map(track_key).as_deref() != Some(k.as_str()) {
                        return;
                    }
                    s.resolving = false;
                    match r {
                        Ok(Some(res)) => {
                            s.current_source = Some(res.source.provider());
                            s.current_preview =
                                matches!(res.source, glyph_protocol::PlaybackSource::Preview { .. });
                            let src = res.source.clone();
                            let tr = res.track.clone();
                            s.with_player(move |p| {
                                p.load(tr, src);
                                p.play();
                            });
                            // Now that this song runs, get the next ones ready
                            // (local yt-dlp only; one batch run, not one wait per song).
                            s.prefetch_upcoming();
                        }
                        Ok(None) => s.toast_info(crate::i18n::t("toast.no_source")),
                        Err(e) => s.toast_info(tf("toast.resolve_error", &[("e", &e)])),
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }

        // Lyrics.
        let b = backend.clone();
        let t = track.clone();
        let k = key.clone();
        cx.spawn(async move |this, cx| {
            let r = guard_async(b.lyrics(&t)).await;
            this.update(cx, |s, cx| {
                if s.lyrics.key.as_deref() != Some(k.as_str()) {
                    return;
                }
                s.lyrics.loading = false;
                match r {
                    Ok(l) => s.lyrics.lyrics = l,
                    Err(e) => s.lyrics.error = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();

        // Cover → palette + ASCII art.
        let b = backend;
        let t = track;
        cx.spawn(async move |this, cx| {
            let Ok(Some(path)) = guard_async(b.cover(&t)).await else { return };
            let p2 = path.clone();
            let (pal, art) = cx
                .background_executor()
                .spawn(async move { (Palette::from_image(&p2), ascii::from_image(&p2, 34, 17)) })
                .await;
            this.update(cx, |s, cx| {
                if s.current.as_ref().map(track_key).as_deref() != Some(key.as_str()) {
                    return;
                }
                if let Some(p) = pal {
                    s.palette = Some(p);
                    s.glow.update(cx, |g, _| g.set_palette(Some(p)));
                }
                s.cover = Some(CoverArt {
                    key: key.clone(),
                    path: Some(path),
                    ascii: art.unwrap_or_else(|| ascii::pattern(&key, 34, 17)),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn toggle_play(&mut self, cx: &mut Context<Self>) {
        if self.current.is_none() {
            let recent = self.library.recent.clone();
            if !recent.is_empty() {
                self.play_list(recent, 0, cx);
            }
            return;
        }
        self.with_player(|p| p.toggle());
        cx.notify();
    }

    pub fn seek_to(&mut self, pos: Duration) {
        self.with_player(|p| p.seek(pos));
    }

    pub fn seek_relative(&mut self, secs: f32) {
        let cur = self.position().as_secs_f32();
        let max = self.duration().map(|d| d.as_secs_f32()).unwrap_or(f32::MAX);
        let to = (cur + secs).clamp(0.0, max);
        self.seek_to(Duration::from_secs_f32(to));
    }

    pub fn set_volume(&mut self, v: f32) {
        let v = v.clamp(0.0, 1.0);
        if v > 0.001 {
            self.unmute_volume = v;
        }
        self.with_player(|p| p.set_volume(v));
    }

    /// Speaker icon: mute, or back to the volume before muting.
    pub fn toggle_mute(&mut self) {
        let muted = self.volume() <= 0.001;
        if !muted {
            self.unmute_volume = self.volume();
        }
        let v = if muted { self.unmute_volume } else { 0.0 };
        self.with_player(|p| p.set_volume(v));
    }

    pub fn next(&mut self, auto: bool, cx: &mut Context<Self>) {
        if !auto && self.library_ok && self.position() < Duration::from_secs(30) {
            if let Some(cur) = self.current.clone() {
                self.library.record_skip(&cur);
            }
        }
        let preloaded = self.preloaded.take();
        match guard(|| if auto { self.queue.next().cloned() } else { self.queue.skip_next().cloned() }) {
            Some(Some(t)) => {
                // Ended on its own and the player already runs the preloaded
                // next song: follow it instead of resolving and loading again.
                let playing = self.player.as_ref().and_then(|p| guard(|| p.state().track)).flatten();
                let adopted = preloaded.filter(|(k, r)| {
                    auto && *k == track_key(&t) && playing.as_ref().is_some_and(|p| p.id == r.id && p.source == r.source)
                });
                self.start_track(t, adopted.map(|(_, r)| r), cx);
            }
            Some(None) => {
                if self.radio {
                    self.start_radio(cx);
                } else if auto {
                    self.toast_info(t("toast.queue_end"));
                }
            }
            None => {
                self.queue_ok = false;
                self.toast_info("[ QUEUE OFFLINE ]");
            }
        }
        cx.notify();
    }

    pub fn previous(&mut self, cx: &mut Context<Self>) {
        if self.position() > Duration::from_secs(3) {
            self.seek_to(Duration::ZERO);
            return;
        }
        if let Some(Some(t)) = guard(|| self.queue.previous().cloned()) {
            self.play_track(t, cx);
        }
    }

    /// `true` if the error was an expired/denied stream URL and a fresh
    /// resolve was started for the current track.
    ///
    /// Deezer signs preview URLs with a ~15 min lifetime, so a URL fetched
    /// during preload can be dead by the time it plays. Only retried once per
    /// track to avoid a loop when the provider itself is unavailable.
    fn retry_expired_preview(&mut self, error: &str, cx: &mut Context<Self>) -> bool {
        // "HTTP 403" when opening, "(403 Forbidden)" from a later range request.
        let refused = ["HTTP 403", "HTTP 401", "403 Forbidden", "401 Unauthorized"].iter().any(|s| error.contains(s));
        if !refused {
            return false;
        }
        let (Some(backend), Some(current)) = (self.backend.clone(), self.current.clone()) else {
            return false;
        };
        if self.retried_expired.replace(current.clone()).is_some() {
            return false;
        }
        let key = track_key(&current);
        let b = backend;
        // Local YouTube URLs expire too (after hours): fetch fresh ones
        // instead of dropping to a preview.
        let youtube = self.current_source == Some(ProviderId::YouTube);
        cx.spawn(async move |this, cx| {
            let r = if youtube {
                guard_async(b.refresh_source(&current)).await
            } else {
                match guard_async(b.refresh_preview(&current)).await {
                    Ok(Some(r)) => Ok(Some(r)),
                    _ => guard_async(b.resolve(&current)).await,
                }
            };
            this.update(cx, |s, cx| {
                if s.current.as_ref().map(track_key).as_deref() != Some(key.as_str()) {
                    return;
                }
                match r {
                    Ok(Some(res)) => {
                        let src = res.source.clone();
                        let tr = res.track.clone();
                        s.with_player(move |p| {
                            p.load(tr, src);
                            p.play();
                        });
                    }
                    Ok(None) => s.toast_info(crate::i18n::t("toast.no_source")),
                    Err(e) => s.toast_info(tf("toast.resolve_error", &[("e", &e)])),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        true
    }

    fn preload_next(&mut self, cx: &mut Context<Self>) {
        let Some(Some(next)) = guard(|| self.queue.peek_next().cloned()) else { return };
        let Some(b) = self.backend.clone() else { return };
        let cur = self.current.as_ref().map(track_key);
        let want = track_key(&next);
        cx.spawn(async move |this, cx| {
            if let Ok(Some(res)) = guard_async(b.resolve(&next)).await {
                this.update(cx, |s, _| {
                    // Resolving can take a while: if another song started or
                    // the queue changed meanwhile, this one is not next anymore.
                    if s.current.as_ref().map(track_key) != cur || s.next_key().as_deref() != Some(want.as_str()) {
                        return;
                    }
                    s.preloaded = Some((want, res.track.clone()));
                    s.with_player(|p| p.preload_next(res.track, res.source));
                })
                .ok();
            }
        })
        .detach();
    }

    fn next_key(&self) -> Option<String> {
        guard(|| self.queue.peek_next().map(track_key)).flatten()
    }

    /// After a queue edit: a preloaded song that is no longer next must not
    /// start after the current one (the player then asks for the new next).
    fn queue_changed(&mut self) {
        let stale = self.preloaded.as_ref().is_some_and(|(k, _)| self.next_key().as_deref() != Some(k.as_str()));
        if stale {
            self.preloaded = None;
            self.with_player(|p| p.drop_next());
        }
    }

    /// Queue finished: continue with similar tracks (Last.fm getSimilar).
    fn start_radio(&mut self, cx: &mut Context<Self>) {
        let (Some(b), Some(seed)) = (self.backend.clone(), self.current.clone()) else { return };
        self.toast(tf("toast.radio", &[("title", &seed.title)]));
        cx.spawn(async move |this, cx| {
            let r = guard_async(b.similar(&seed)).await;
            this.update(cx, |s, cx| match r {
                Ok(tracks) if !tracks.is_empty() => {
                    let tracks: Vec<Track> = tracks.into_iter().filter(|t| !s.is_hidden_artist(&t.artist)).collect();
                    if tracks.is_empty() {
                        s.toast_info(t("toast.radio_empty"));
                    } else {
                        s.play_list(tracks, 0, cx)
                    }
                }
                Ok(_) => s.toast_info(t("toast.radio_empty")),
                Err(e) => s.toast_info(tf("toast.radio_error", &[("e", &e)])),
            })
            .ok();
        })
        .detach();
    }

    pub fn toggle_shuffle(&mut self, cx: &mut Context<Self>) {
        let on = !self.shuffle();
        guard(|| self.queue.set_shuffle(on));
        self.queue_changed();
        cx.notify();
    }

    pub fn cycle_repeat(&mut self, cx: &mut Context<Self>) {
        let next = match self.repeat() {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        };
        guard(|| self.queue.set_repeat(next));
        self.queue_changed();
        cx.notify();
    }

    pub fn toggle_mini(&mut self, cx: &mut Context<Self>) {
        self.mini_on_top = !self.mini_on_top;
        let on_top = self.mini_on_top;
        self.with_player(|p| {
            p.set_mini_player_options(MiniPlayerOptions { always_on_top: on_top, position: None })
        });
        cx.notify();
    }

    pub fn queue_add(&mut self, t: Track, next: bool, cx: &mut Context<Self>) {
        let title = t.title.clone();
        let ok = if next {
            guard(|| self.queue.play_next(t))
        } else {
            guard(|| self.queue.push(t))
        };
        match ok {
            Some(()) => self.toast_info(tf(if next { "toast.play_next" } else { "toast.queued" }, &[("title", &title)])),
            None => self.toast_info("[ QUEUE OFFLINE ]"),
        }
        self.queue_changed();
        cx.notify();
    }

    pub fn queue_remove(&mut self, i: usize, cx: &mut Context<Self>) {
        guard(|| self.queue.remove(i));
        self.queue_changed();
        cx.notify();
    }

    pub fn queue_move(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        guard(|| self.queue.move_item(from, to));
        self.queue_changed();
        cx.notify();
    }

    pub fn toggle_like(&mut self, t: &Track, cx: &mut Context<Self>) {
        if !self.library_ok {
            self.toast_info(crate::i18n::t("status.library_offline"));
            return;
        }
        if guard(|| self.library.toggle_like(t)).is_some() {
            self.save_library(cx);
        }
        cx.notify();
    }

    pub fn delete_playlist(&mut self, id: &str, cx: &mut Context<Self>) {
        if guard(|| self.library.delete_playlist(id)).is_some() {
            self.save_library(cx);
        }
        if self.open_list == Some(ListRef::User(id.to_string())) {
            self.history.retain(|r| r.list != Some(ListRef::User(id.to_string())));
            let back = self.history.pop().unwrap_or(Route { view: View::Library, list: None, artist: None });
            self.restore(back, cx);
        }
        cx.notify();
    }

    /// Adds several songs at once (one toast).
    pub fn add_many_to_playlist(&mut self, id: &str, tracks: Vec<Track>, cx: &mut Context<Self>) {
        let mut added = Vec::new();
        let mut name = String::new();
        if let Some(p) = self.library.playlists.iter_mut().find(|p| p.id == id) {
            name = p.name.clone();
            for t in tracks {
                if !p.tracks.iter().any(|x| x.id == t.id && x.source == t.source) {
                    p.tracks.push(t.clone());
                    added.push(t);
                }
            }
        }
        if !added.is_empty() {
            self.save_library(cx);
            self.toast(tf("toast.added_n_to_playlist", &[("n", &added.len()), ("name", &name)]));
            self.auto_download_added(id, added, cx);
        }
        cx.notify();
    }

    pub fn add_to_playlist(&mut self, id: &str, t: Track, cx: &mut Context<Self>) {
        if let Some(p) = self.library.playlists.iter_mut().find(|p| p.id == id) {
            if !p.tracks.iter().any(|x| x.id == t.id && x.source == t.source) {
                p.tracks.push(t.clone());
                let name = p.name.clone();
                self.save_library(cx);
                self.toast(tf("toast.added_to_playlist", &[("name", &name)]));
                self.auto_download_added(id, vec![t], cx);
            }
        }
        cx.notify();
    }

    pub fn is_auto_download(&self, id: &str) -> bool {
        self.library_ok && self.library.playlist(id).is_some_and(|p| p.auto_download)
    }

    /// Turns "download new songs automatically" on or off for a playlist.
    /// Songs already in it stay as they are ("Download all" covers those).
    pub fn toggle_auto_download(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.library_ok {
            self.toast_info(t("status.library_offline"));
            return;
        }
        let Some(p) = self.library.playlist_mut(id) else { return };
        p.auto_download = !p.auto_download;
        let on = p.auto_download;
        self.save_library(cx);
        self.toast_info(t(if on { "toast.auto_dl_on" } else { "toast.auto_dl_off" }));
        cx.notify();
    }

    /// Songs just added to playlist `id` (here or from another device):
    /// downloads them if the playlist downloads new songs automatically.
    pub(crate) fn auto_download_added(&mut self, id: &str, added: Vec<Track>, cx: &mut Context<Self>) {
        if added.is_empty() || !self.is_auto_download(id) {
            return;
        }
        self.download_tracks_with(added, true, cx);
    }

    pub fn share(&mut self, t: &Track, cx: &mut Context<Self>) {
        match &t.permalink {
            Some(url) => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(url.clone()));
                self.toast_sub(crate::i18n::t("toast.link_copied_plain"), t.title.clone());
            }
            None => self.toast_info(crate::i18n::t("toast.no_link")),
        }
    }

    pub fn shift_lyrics_offset(&mut self, delta_ms: i64, cx: &mut Context<Self>) {
        let Some(t) = self.current.clone() else { return };
        let cur = self.lyrics_offset();
        if self.library_ok && guard(|| self.library.set_lyrics_offset(&t, cur + delta_ms)).is_some()
        {
            self.save_library(cx);
        }
        cx.notify();
    }

    // ------------------------------------------------------------ lists & artists

    fn route(&self) -> Route {
        Route { view: self.view, list: self.open_list.clone(), artist: self.open_artist.clone() }
    }

    fn push_route(&mut self) {
        let r = self.route();
        if self.history.last() != Some(&r) {
            self.history.push(r);
        }
        if self.history.len() > 50 {
            self.history.remove(0);
        }
    }

    fn restore(&mut self, r: Route, cx: &mut Context<Self>) {
        self.view = r.view;
        self.main_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        self.open_list = r.list;
        self.open_artist = r.artist.clone();
        if let Some(a) = r.artist {
            self.load_artist(a, cx);
        }
        self.ctx_menu = None;
        cx.notify();
    }

    pub fn go_back(&mut self, cx: &mut Context<Self>) {
        if let Some(r) = self.history.pop() {
            self.restore(r, cx);
        }
    }

    /// Opens a song list (never starts playback).
    pub fn open_list(&mut self, l: ListRef, cx: &mut Context<Self>) {
        if self.view == View::List && self.open_list.as_ref() == Some(&l) {
            return;
        }
        self.push_route();
        self.view = View::List;
        self.main_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        if self.library_ok && guard(|| self.library.push_recent_list(l.key())).is_some() {
            self.save_library(cx);
        }
        self.open_list = Some(l);
        self.open_artist = None;
        self.ctx_menu = None;
        cx.notify();
    }

    pub fn list_tracks(&self, l: &ListRef) -> Vec<Track> {
        match l {
            ListRef::Liked => self.library.liked.clone(),
            ListRef::Recent => self.library.recent.clone(),
            ListRef::User(id) => self.library.playlist(id).map(|p| p.tracks.clone()).unwrap_or_default(),
            ListRef::Album { tracks, .. } => tracks.clone(),
            ListRef::ArtistLiked(name) => self.liked_by(name),
            ListRef::Downloads => self.backend.as_ref().map(|b| b.downloaded_tracks()).unwrap_or_default(),
        }
    }

    /// Copies a short link for the list (public server) or its share code
    /// (songs, name) to the clipboard.
    pub fn share_list(&mut self, l: &ListRef, cx: &mut Context<Self>) {
        let tracks = self.list_tracks(l);
        if tracks.is_empty() {
            self.toast_info(t("toast.share_empty"));
            cx.notify();
            return;
        }
        let title = self.list_title(l);
        let Some(b) = self.backend.clone() else {
            let code = glyph_core::share::encode(&title, &tracks);
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(code));
            self.toast_sub(t("toast.share_copied"), title);
            cx.notify();
            return;
        };
        // Artist albums carry no author; a list opened from a link keeps its link.
        let kind = if matches!(l, ListRef::Album { .. }) { "album" } else { "playlist" };
        let origin = match l {
            ListRef::User(id) => self.library.playlist(id).and_then(|p| p.origin.clone()),
            _ => None,
        };
        cx.spawn(async move |this, cx| {
            let text = b.share_text(&title, &tracks, kind, origin).await;
            this.update(cx, |s, cx| {
                let key = if glyph_core::share::link_id(&text).is_some() { "toast.share_link_copied" } else { "toast.share_copied" };
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                s.toast_sub(t(key), title);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Imports a pasted share code (or short link) as a new playlist and opens it.
    pub fn import_share_code(&mut self, code: &str, cx: &mut Context<Self>) -> bool {
        if glyph_core::share::link_id(code).is_some() {
            let Some(b) = self.backend.clone() else {
                self.toast_info(t("status.library_offline"));
                cx.notify();
                return true;
            };
            let link = code.to_string();
            cx.spawn(async move |this, cx| {
                let r = b.resolve_share_link(&link).await;
                this.update(cx, |s, cx| match r {
                    Ok(p) if glyph_core::share::is_code(&p.code) => {
                        let author = p.author.filter(|_| p.kind != "album");
                        s.import_code(&p.code, Some(link.clone()), author, cx);
                    }
                    Ok(_) => {
                        s.toast_info(t("toast.share_link_failed"));
                        cx.notify();
                    }
                    Err(e) => {
                        tracing::info!("playlist link failed: {e:#}");
                        s.toast_info(t("toast.share_link_failed"));
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
            return true;
        }
        if !glyph_core::share::is_code(code) {
            return false;
        }
        self.import_code(code, None, None, cx);
        true
    }

    /// Adds the playlist of a share code; `origin` / `shared_by` come from a
    /// short link (its URL and the author's handle).
    fn import_code(&mut self, code: &str, origin: Option<String>, shared_by: Option<String>, cx: &mut Context<Self>) {
        match glyph_core::share::decode(code) {
            Ok((name, tracks)) if self.library_ok => {
                let n = tracks.len();
                let id = {
                    let p = self.library.create_playlist(&name);
                    p.tracks = tracks;
                    p.origin = origin;
                    p.shared_by = shared_by;
                    p.id.clone()
                };
                self.save_library(cx);
                self.toast_sub(tf("toast.share_imported", &[("n", &n)]), name);
                self.open_list(ListRef::User(id), cx);
            }
            Ok(_) => self.toast_info(t("status.library_offline")),
            Err(e) => self.toast_info(format!("{e:#}")),
        }
        cx.notify();
    }

    /// "2019 · EP"-style label for an album; looked up once, guessed from the
    /// song count until (or unless) the catalog knows it.
    pub fn album_label(&mut self, title: &str, artist: &str, n: usize, cx: &mut Context<Self>) -> String {
        use glyph_core::catalog::AlbumKind;
        let key = format!("{}|{}", title.to_lowercase(), artist.to_lowercase());
        if !self.album_meta.contains_key(&key) {
            self.album_meta.insert(key.clone(), None);
            if let Some(b) = self.backend.clone() {
                let (t2, a2, k2) = (title.to_string(), artist.to_string(), key.clone());
                cx.spawn(async move |this, cx| {
                    let r = guard_async(b.album_meta(&t2, &a2)).await;
                    if let Ok(Some(m)) = r {
                        this.update(cx, |s, cx| {
                            s.album_meta.insert(k2, Some(m));
                            cx.notify();
                        })
                        .ok();
                    }
                })
                .detach();
            }
        }
        let meta = self.album_meta.get(&key).cloned().flatten();
        let kind = meta.as_ref().map(|m| m.kind).unwrap_or(AlbumKind::from_count(n));
        let kind = match kind {
            AlbumKind::Album => t("album.kind.album"),
            AlbumKind::Ep => t("album.kind.ep"),
            AlbumKind::Single => t("album.kind.single"),
            AlbumKind::Compilation => t("album.kind.compilation"),
        };
        match meta.and_then(|m| m.year) {
            Some(y) => format!("{y} · {kind}"),
            None => kind.to_string(),
        }
    }

    /// The taste profile from the library (cheap; built on demand).
    pub fn taste(&self) -> glyph_core::taste::Taste {
        if self.library_ok {
            glyph_core::taste::Taste::from_library(&self.library)
        } else {
            Default::default()
        }
    }

    /// Not filtered out by "Hide explicit songs".
    pub fn allowed(&self, t: &Track) -> bool {
        !(t.explicit && self.config.audio.hide_explicit)
    }

    /// Loads "Recommended for you" and "Popular right now" for Home.
    pub fn load_feeds(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        if self.feeds_loading {
            return;
        }
        self.feeds_loading = true;
        // Seeds: one song for each of the user's top artists, then liked songs.
        let taste = self.taste();
        let mut seeds: Vec<Track> = Vec::new();
        let pool: Vec<Track> = self.library.liked.iter().chain(self.library.recent.iter()).cloned().collect();
        for a in taste.top_artists(4) {
            if let Some(t) = pool.iter().find(|t| glyph_core::taste::credited(&t.artist).contains(&a)) {
                if !seeds.iter().any(|s| s.id == t.id && s.source == t.source) {
                    seeds.push(t.clone());
                }
            }
        }
        for t in self.library.liked.iter().take(2) {
            if !seeds.iter().any(|s| s.id == t.id && s.source == t.source) {
                seeds.push(t.clone());
            }
        }
        seeds.truncate(5);
        cx.spawn(async move |this, cx| {
            let trending = b.trending(30).await.unwrap_or_default();
            this.update(cx, |s, cx| {
                s.trending = trending.into_iter().filter(|t| s.allowed(t) && !s.is_hidden_artist(&t.artist)).take(20).collect();
                cx.notify();
            })
            .ok();
            // Similar songs per seed (via the server); songs suggested by
            // several seeds and by liked artists come first.
            let mut found: Vec<(Track, f32)> = Vec::new();
            for seed in &seeds {
                let sim = b.similar(seed).await.unwrap_or_default();
                for (i, t) in sim.into_iter().take(25).enumerate() {
                    let w = 1.0 - i as f32 / 30.0;
                    let key = glyph_core::dedupe::track_key(&t);
                    match found.iter_mut().find(|(x, _)| glyph_core::dedupe::track_key(x) == key) {
                        Some(f) => f.1 += w,
                        None => found.push((t, w)),
                    }
                }
            }
            this.update(cx, |s, cx| {
                let taste = s.taste();
                let recent: HashSet<String> = s.library.recent.iter().take(30).map(glyph_core::dedupe::track_key).collect();
                let mut v: Vec<(Track, f32)> = found
                    .into_iter()
                    .filter(|(t, _)| s.allowed(t) && !taste.is_hidden(&t.artist) && !taste.is_liked(t) && !recent.contains(&glyph_core::dedupe::track_key(t)))
                    .map(|(t, w)| {
                        let sc = w + taste.artist_weight(&t.artist) * 0.8;
                        (t, sc)
                    })
                    .collect();
                v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                // At most two songs per artist, so it does not become one artist's page.
                let mut per: std::collections::HashMap<String, usize> = Default::default();
                s.recs = v
                    .into_iter()
                    .filter(|(t, _)| {
                        let n = per.entry(glyph_core::dedupe::normalize_artist(&t.artist)).or_default();
                        *n += 1;
                        *n <= 2
                    })
                    .map(|(t, _)| t)
                    .take(16)
                    .collect();
                s.feeds_loading = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn is_downloaded(&self, t: &Track) -> bool {
        self.backend.as_ref().is_some_and(|b| b.is_downloaded(t))
    }

    pub fn is_downloading(&self, t: &Track) -> bool {
        !self.downloading.is_empty() && self.downloading.contains(&track_key(t))
    }

    /// Download badge state of a song (ring while busy, check when saved).
    pub fn download_view(&self, t: &Track) -> crate::ui::widgets::DlView {
        use crate::ui::widgets::{DlView, DL_DONE_ANIM_S};
        if self.is_downloading(t) {
            // Waiting for its turn: empty ring; running with unknown size: spinner.
            return match self.backend.as_ref().and_then(|b| b.download_progress(t)) {
                None => DlView::Busy(Some(0.0)),
                Some(f) => DlView::Busy(f),
            };
        }
        if !self.is_downloaded(t) {
            return DlView::None;
        }
        let age = if self.download_done.is_empty() {
            None
        } else {
            self.download_done.get(&track_key(t)).map(|at| at.elapsed().as_secs_f32()).filter(|a| *a < DL_DONE_ANIM_S)
        };
        DlView::Done(age)
    }

    /// One badge for several songs: the share already fetched while any of
    /// them downloads, the check once all are saved.
    pub fn downloads_view(&self, tracks: &[Track]) -> crate::ui::widgets::DlView {
        use crate::ui::widgets::DlView;
        if tracks.is_empty() {
            return DlView::None;
        }
        let views: Vec<DlView> = tracks.iter().map(|t| self.download_view(t)).collect();
        if views.iter().any(|v| matches!(v, DlView::Busy(_))) {
            let sum: f32 = views
                .iter()
                .map(|v| match v {
                    DlView::Done(_) => 1.0,
                    DlView::Busy(f) => f.unwrap_or(0.0),
                    DlView::None => 0.0,
                })
                .sum();
            return DlView::Busy(Some(sum / views.len() as f32));
        }
        if views.iter().all(|v| matches!(v, DlView::Done(_))) {
            // Animate when the last one just finished.
            let age = views.iter().filter_map(|v| if let DlView::Done(a) = v { *a } else { None }).reduce(f32::min);
            return DlView::Done(age);
        }
        DlView::None
    }

    /// Downloads the songs one after another; songs no source offers for
    /// download are skipped and counted in the closing toast.
    pub fn download_tracks(&mut self, tracks: Vec<Track>, cx: &mut Context<Self>) {
        self.download_tracks_with(tracks, false, cx);
    }

    /// `auto`: started by a playlist that downloads new songs by itself, so
    /// it starts silently and only reports what was saved or failed.
    fn download_tracks_with(&mut self, tracks: Vec<Track>, auto: bool, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else {
            if !auto {
                self.toast_info(t("toast.dl_unready"));
            }
            cx.notify();
            return;
        };
        let todo: Vec<Track> = tracks.into_iter().filter(|tr| !self.is_downloaded(tr) && !self.is_downloading(tr)).collect();
        if todo.is_empty() {
            if !auto {
                self.toast_info(t("toast.dl_already"));
            }
            cx.notify();
            return;
        }
        for tr in &todo {
            self.downloading.insert(track_key(tr));
        }
        let total = todo.len();
        let first_title = todo[0].title.clone();
        if !auto {
            self.toast(if total == 1 {
                tf("toast.dl_started_one", &[("title", &first_title)]).to_string()
            } else {
                tf("toast.dl_started", &[("n", &total)]).to_string()
            });
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (mut ok, mut missing, mut failed) = (0usize, 0usize, 0usize);
            for tr in todo {
                let saved = match b.download(&tr).await {
                    Ok(Some(_)) => {
                        ok += 1;
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
                let k = track_key(&tr);
                this.update(cx, |s, cx| {
                    s.downloading.remove(&k);
                    if saved {
                        s.download_done.retain(|_, at| at.elapsed() < Duration::from_secs(5));
                        s.download_done.insert(k, Instant::now());
                    }
                    cx.notify();
                })
                .ok();
            }
            // Songs no source offers for download are expected in an
            // automatic run; it says nothing unless something happened.
            if auto && ok + failed == 0 {
                return;
            }
            let msg = if ok + failed == 0 {
                t("toast.dl_none").to_string()
            } else if total == 1 && ok == 1 {
                tf("toast.dl_done_one", &[("title", &first_title)]).to_string()
            } else {
                let mut parts = Vec::new();
                if ok > 0 {
                    parts.push(tf("toast.dl_done", &[("n", &ok)]).to_string());
                }
                if missing > 0 {
                    parts.push(tf("toast.dl_unavailable", &[("n", &missing)]).to_string());
                }
                if failed > 0 {
                    parts.push(tf("toast.dl_failed", &[("n", &failed)]).to_string());
                }
                parts.join(" · ")
            };
            this.update(cx, |s, cx| {
                s.toast(msg);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Deletes the downloaded files of `tracks`.
    pub fn remove_downloads(&mut self, tracks: Vec<Track>, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let n = tracks.iter().filter(|tr| b.remove_download(tr)).count();
        if n > 0 {
            self.toast(if n == 1 { t("toast.dl_removed_one").to_string() } else { tf("toast.dl_removed", &[("n", &n)]).to_string() });
        }
        cx.notify();
    }

    pub fn is_following(&self, name: &str) -> bool {
        self.library_ok && self.library.is_following(name)
    }

    pub fn toggle_follow(&mut self, name: &str, track: Option<Track>, cx: &mut Context<Self>) {
        if !self.library_ok {
            self.toast_info(t("status.library_offline"));
            return;
        }
        if let Some(now) = guard(|| self.library.toggle_follow(name, track)) {
            self.save_library(cx);
            self.toast(tf(if now { "toast.following" } else { "toast.unfollowed" }, &[("name", &name)]));
        }
        cx.notify();
    }

    pub fn is_saved(&self, l: &ListRef) -> bool {
        self.library_ok && self.library.is_saved(&l.key())
    }

    /// Heart on an album: save it to / remove it from the library.
    pub fn toggle_saved(&mut self, l: &ListRef, cx: &mut Context<Self>) {
        if !self.library_ok {
            self.toast_info(t("status.library_offline"));
            return;
        }
        if let Some(saved) = guard(|| self.library.toggle_saved(l.key())) {
            self.save_library(cx);
            self.toast(if saved { t("toast.saved_to_library") } else { t("toast.removed_from_library") });
        }
        cx.notify();
    }

    /// Recently opened lists that still exist (sidebar).
    pub fn recent_lists(&self, max: usize) -> Vec<ListRef> {
        self.library
            .recent_lists
            .iter()
            .filter(|k| match k {
                glyph_core::ListKey::Playlist { id } => self.library.playlist(id).is_some(),
                _ => true,
            })
            .take(max)
            .map(ListRef::from_key)
            .collect()
    }

    /// Liked songs (co-)credited to `name`.
    pub fn liked_by(&self, name: &str) -> Vec<Track> {
        self.library.liked.iter().filter(|t| by_artist(name, &t.artist)).cloned().collect()
    }

    pub fn list_title(&self, l: &ListRef) -> String {
        match l {
            ListRef::Liked => t("library.liked").to_string(),
            ListRef::Recent => t("library.recent").to_string(),
            ListRef::User(id) => self.library.playlist(id).map(|p| p.name.clone()).unwrap_or_default(),
            ListRef::Album { title, .. } => title.clone(),
            ListRef::ArtistLiked(_) => t("library.liked").to_string(),
            ListRef::Downloads => t("library.downloads").to_string(),
        }
    }

    /// Opens the artist profile (first artist of a "A, B" credit).
    pub fn open_artist(&mut self, name: &str, cx: &mut Context<Self>) {
        let name = primary_artist(name);
        if name.is_empty() || (self.view == View::Artist && self.open_artist.as_deref() == Some(name.as_str())) {
            return;
        }
        self.push_route();
        self.view = View::Artist;
        self.main_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        self.open_artist = Some(name.clone());
        self.open_list = None;
        self.ctx_menu = None;
        self.load_artist(name, cx);
        cx.notify();
    }

    fn load_artist(&mut self, name: String, cx: &mut Context<Self>) {
        if self.artists.contains_key(&name) {
            return;
        }
        let Some(b) = self.backend.clone() else { return };
        // Songs: a normal search for the name, keeping the artist's own songs.
        let songs = {
            let (b, name) = (b.clone(), name.clone());
            cx.spawn(async move |this, cx| {
                let mut found: Vec<Track> = Vec::new();
                if let Some(mut stream) = guard(|| b.search(&name)) {
                    use futures::StreamExt;
                    while let Some(ev) = stream.next().await {
                        match ev {
                            SearchEvent::Results { tracks, .. } => {
                                let mut changed = false;
                                for tr in tracks {
                                    let dup = found.iter().any(|x| {
                                        glyph_core::dedupe::normalize_title(&x.title)
                                            == glyph_core::dedupe::normalize_title(&tr.title)
                                    });
                                    if by_artist(&name, &tr.artist) && !dup {
                                        found.push(tr);
                                        changed = true;
                                    }
                                }
                                if changed {
                                    let list = found.clone();
                                    let n = name.clone();
                                    this.update(cx, |s, cx| {
                                        if let Some(p) = s.artists.get_mut(&n) {
                                            p.tracks = list;
                                            cx.notify();
                                        }
                                    })
                                    .ok();
                                }
                            }
                            SearchEvent::Done => break,
                            _ => {}
                        }
                    }
                }
                this.update(cx, |s, cx| {
                    if let Some(p) = s.artists.get_mut(&name) {
                        p.loading = false;
                    }
                    cx.notify();
                })
                .ok();
            })
        };
        let stats = {
            let (name, b) = (name.clone(), b.clone());
            cx.spawn(async move |this, cx| {
                let r = guard_async(b.artist_stats(&name)).await;
                this.update(cx, |s, cx| {
                    if let (Some(p), Ok(Some(st))) = (s.artists.get_mut(&name), r) {
                        p.stats = Some(st);
                        cx.notify();
                    }
                })
                .ok();
            })
        };
        let info = {
            let (name, lang) = (name.clone(), self.lang.code());
            cx.spawn(async move |this, cx| {
                let r = guard_async(b.artist_info(&name, lang)).await;
                this.update(cx, |s, cx| {
                    if let Some(p) = s.artists.get_mut(&name) {
                        p.info_loading = false;
                        if let Ok(Some(i)) = r {
                            p.info = Some(i);
                        }
                    }
                    cx.notify();
                })
                .ok();
            })
        };
        self.artists.insert(
            name.clone(),
            ArtistPage {
                name,
                tracks: Vec::new(),
                loading: true,
                info: None,
                info_loading: true,
                stats: None,
                show_all: false,
                about_open: false,
                _tasks: vec![songs, info, stats],
            },
        );
    }

    /// Opens the album of `seed` and fills in its other songs: from the
    /// library if saved, otherwise by searching "artist album" in the background.
    pub fn open_album(&mut self, title: &str, artist: &str, seed: Track, cx: &mut Context<Self>) {
        let artist = primary_artist(artist);
        let probe = ListRef::Album { title: title.to_string(), artist: artist.clone(), tracks: vec![seed] };
        if let Some(saved) = self.library.saved_albums.iter().find(|k| k.same(&probe.key())).cloned() {
            self.open_list(ListRef::from_key(&saved), cx);
            return;
        }
        self.open_list(probe, cx);
        let Some(b) = self.backend.clone() else { return };
        let (title, q) = (title.to_string(), format!("{artist} {title}"));
        self.album_task = Some(cx.spawn(async move |this, cx| {
            let Some(mut stream) = guard(|| b.search(&q)) else { return };
            use futures::StreamExt;
            while let Some(ev) = stream.next().await {
                match ev {
                    SearchEvent::Results { tracks, .. } => {
                        let hits: Vec<Track> = tracks
                            .into_iter()
                            .filter(|t| t.album.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(&title)) && by_artist(&artist, &t.artist))
                            .collect();
                        if hits.is_empty() {
                            continue;
                        }
                        let (t2, a2) = (title.clone(), artist.clone());
                        this.update(cx, |s, cx| {
                            if let Some(ListRef::Album { title, artist, tracks }) = s.open_list.as_mut() {
                                if title.eq_ignore_ascii_case(&t2) && artist.eq_ignore_ascii_case(&a2) {
                                    for h in hits {
                                        let n = glyph_core::dedupe::normalize_title(&h.title);
                                        if !tracks.iter().any(|x| glyph_core::dedupe::normalize_title(&x.title) == n) {
                                            tracks.push(h);
                                        }
                                    }
                                    cx.notify();
                                }
                            }
                        })
                        .ok();
                    }
                    SearchEvent::Done => break,
                    _ => {}
                }
            }
        }));
    }

    /// Plays the `i`-th recently played song (taskbar jump list / dock menu).
    pub fn play_recent(&mut self, i: usize, cx: &mut Context<Self>) {
        let list = self.library.recent.clone();
        if i < list.len() {
            self.play_list(list, i, cx);
        }
    }

    /// Jump list / dock entry `n`: 0..=4 recently played, 5 play/pause,
    /// 6 next, 7 previous.
    fn dock_action(&mut self, n: usize, cx: &mut Context<Self>) {
        match n {
            0..=4 => self.play_recent(n, cx),
            5 => self.toggle_play(cx),
            6 => self.next(false, cx),
            7 => self.previous(cx),
            _ => {}
        }
    }

    /// Recently played songs + transport in the taskbar jump list (Windows)
    /// and the dock menu (macOS). Linux has no such menu; nothing happens there.
    fn update_dock_menu(&mut self, cx: &mut Context<Self>) {
        if !self.library_ok {
            return;
        }
        let keys: Vec<String> = self.library.recent.iter().take(5).map(track_key).collect();
        if keys == self.dock_keys {
            return;
        }
        self.dock_keys = keys;
        // Windows: own jump list with a "Recently played" category.
        #[cfg(windows)]
        {
            use crate::jumplist::{update, Entry};
            let recent = self
                .library
                .recent
                .iter()
                .take(5)
                .enumerate()
                .map(|(i, tr)| Entry { label: format!("{} · {}", tr.title, tr.artist), action: i })
                .collect();
            let tasks = vec![
                Entry { label: t("dock.play_pause").to_string(), action: 5 },
                Entry { label: t("dock.next").to_string(), action: 6 },
                Entry { label: t("dock.previous").to_string(), action: 7 },
            ];
            update(t("library.recent").to_string(), recent, tasks);
            let _ = cx;
        }
        // macOS: dock menu with in-process actions.
        #[cfg(not(windows))]
        self.set_platform_dock_menu(cx);
    }

    #[cfg(not(windows))]
    fn set_platform_dock_menu(&mut self, cx: &mut Context<Self>) {
        use crate::ui::{NextTrack, PlayRecent0, PlayRecent1, PlayRecent2, PlayRecent3, PlayRecent4, PrevTrack, TogglePlay};
        let mut items: Vec<gpui::MenuItem> = Vec::new();
        for (i, tr) in self.library.recent.iter().take(5).enumerate() {
            let name = format!("{} · {}", tr.title, tr.artist);
            items.push(match i {
                0 => gpui::MenuItem::action(name, PlayRecent0),
                1 => gpui::MenuItem::action(name, PlayRecent1),
                2 => gpui::MenuItem::action(name, PlayRecent2),
                3 => gpui::MenuItem::action(name, PlayRecent3),
                _ => gpui::MenuItem::action(name, PlayRecent4),
            });
        }
        items.push(gpui::MenuItem::action(t("dock.play_pause"), TogglePlay));
        items.push(gpui::MenuItem::action(t("dock.next"), NextTrack));
        items.push(gpui::MenuItem::action(t("dock.previous"), PrevTrack));
        cx.set_dock_menu(items);
    }

    /// Plays `tracks` from a random song with shuffle on.
    pub fn play_shuffled(&mut self, tracks: Vec<Track>, cx: &mut Context<Self>) {
        if tracks.is_empty() {
            return;
        }
        let i = (self.started.elapsed().as_nanos() as usize) % tracks.len();
        let track = tracks[i].clone();
        guard(|| {
            self.queue.set(tracks, i);
            self.queue.set_shuffle(true);
        });
        self.play_track(track, cx);
    }

    /// Adds every song to the end of the queue.
    pub fn queue_all(&mut self, tracks: Vec<Track>, cx: &mut Context<Self>) {
        let n = tracks.len();
        for tr in tracks {
            guard(|| self.queue.push(tr));
        }
        self.queue_changed();
        self.toast(tf("toast.queued_n", &[("n", &n)]));
        cx.notify();
    }

    pub fn open_ctx(&mut self, track: Track, pos: gpui::Point<gpui::Pixels>, from: Option<(ListRef, usize)>, cx: &mut Context<Self>) {
        self.ctx_menu = Some(CtxMenu { track, pos, from, playlists: false, list: None, artist: None });
        self.ctx_opened = Instant::now();
        self.ctx_closing = None;
        cx.notify();
    }

    /// "More" menu of an artist page.
    pub fn open_artist_ctx(&mut self, name: &str, pos: gpui::Point<gpui::Pixels>, cx: &mut Context<Self>) {
        let songs = self.artists.get(name).map(|p| p.tracks.clone()).unwrap_or_default();
        let track = songs.first().cloned().unwrap_or(Track {
            id: String::new(),
            source: ProviderId::Audius,
            title: String::new(),
            artist: name.to_string(),
            album: None,
            duration_ms: None,
            cover_url: None,
            isrc: None,
            permalink: None,
            external_ids: Default::default(),
            explicit: false,
        });
        self.ctx_menu = Some(CtxMenu { track, pos, from: None, playlists: false, list: None, artist: Some(name.to_string()) });
        self.ctx_opened = Instant::now();
        self.ctx_closing = None;
        cx.notify();
    }

    /// Copies a link for the artist: the Wikipedia article if known, else
    /// their top song on its platform.
    pub fn copy_artist_link(&mut self, name: &str, cx: &mut Context<Self>) {
        let page = self.artists.get(name);
        let url = page
            .and_then(|p| p.info.as_ref().map(|i| i.url.clone()).filter(|u| !u.is_empty()))
            .or_else(|| page.and_then(|p| p.tracks.iter().find_map(|t| t.permalink.clone())));
        match url {
            Some(u) => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(u));
                self.toast_sub(t("toast.link_copied_plain"), name.to_string());
            }
            None => self.toast_info(t("toast.no_link")),
        }
    }

    pub fn is_hidden_artist(&self, credit: &str) -> bool {
        self.library_ok && !self.library.hidden_artists.is_empty() && self.library.hidden_artists.iter().any(|h| by_artist(h, credit))
    }

    /// "Don't show this artist" / "Show again".
    pub fn toggle_hidden_artist(&mut self, name: &str, cx: &mut Context<Self>) {
        if !self.library_ok {
            return;
        }
        if let Some(hidden) = guard(|| self.library.toggle_hidden(name)) {
            self.save_library(cx);
            if hidden {
                let n = name.to_string();
                self.search.results.retain(|t| !by_artist(&n, &t.artist));
                self.toast(tf("toast.artist_hidden", &[("name", &name)]));
            } else {
                self.toast(tf("toast.artist_shown", &[("name", &name)]));
            }
        }
        cx.notify();
    }

    /// Right-click on an album / playlist tile.
    pub fn open_list_ctx(&mut self, l: ListRef, pos: gpui::Point<gpui::Pixels>, cx: &mut Context<Self>) {
        let first = self.list_tracks(&l).into_iter().next();
        let is_user = matches!(l, ListRef::User(_));
        // Empty lists only have something to offer when they are the user's own.
        let track = match first {
            Some(t) => t,
            None if is_user => Track {
                id: String::new(),
                source: ProviderId::Audius,
                title: String::new(),
                artist: String::new(),
                album: None,
                duration_ms: None,
                cover_url: None,
                isrc: None,
                permalink: None,
                external_ids: Default::default(),
                explicit: false,
            },
            None => return,
        };
        self.ctx_menu = Some(CtxMenu { track, pos, from: None, playlists: false, list: Some(l), artist: None });
        self.ctx_opened = Instant::now();
        self.ctx_closing = None;
        cx.notify();
    }

    pub fn close_ctx(&mut self, cx: &mut Context<Self>) {
        if self.ctx_menu.is_some() {
            self.begin_ctx_close();
            cx.notify();
        }
    }

    /// Lets the open menu fade out (the caller notifies).
    pub fn begin_ctx_close(&mut self) {
        if let Some(m) = self.ctx_menu.take() {
            self.ctx_closing = Some((m, Instant::now()));
        }
    }

    /// Removes the song at `i` from a list (unlike for Liked Songs).
    pub fn remove_from_list(&mut self, l: &ListRef, i: usize, cx: &mut Context<Self>) {
        match l {
            ListRef::Liked => {
                if let Some(tr) = self.library.liked.get(i).cloned() {
                    self.toggle_like(&tr, cx);
                }
            }
            ListRef::Recent => {
                if i < self.library.recent.len() {
                    self.library.recent.remove(i);
                    self.save_library(cx);
                }
            }
            ListRef::User(id) => {
                if guard(|| self.library.remove_from_playlist(id, i)).is_some() {
                    self.save_library(cx);
                }
            }
            ListRef::ArtistLiked(name) => {
                if let Some(tr) = self.liked_by(name).get(i).cloned() {
                    self.toggle_like(&tr, cx);
                }
            }
            ListRef::Album { .. } => {}
            ListRef::Downloads => {
                if let Some(tr) = self.list_tracks(l).get(i).cloned() {
                    self.remove_downloads(vec![tr], cx);
                }
            }
        }
        cx.notify();
    }

    /// Opens the name dialog for a new playlist (with `track` added) or for
    /// renaming playlist `rename`.
    pub fn open_name_dialog(&mut self, tracks: Vec<Track>, rename: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let current = rename.as_ref().and_then(|id| self.library.playlist(id)).map(|p| p.name.clone()).unwrap_or_default();
        self.playlist_name.update(cx, |i, cx| i.set_value(current, cx));
        self.ctx_menu = None;
        self.name_dialog = Some(NameDialog { tracks, rename, eq_preset: false });
        let f = self.playlist_name.read(cx).focus.clone();
        window.focus(&f, cx);
        cx.notify();
    }

    /// Lets the user pick a picture for playlist `id` (file dialog); it is
    /// copied into the data dir as a 512 px JPEG.
    pub fn pick_playlist_cover(&mut self, id: String, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t("dialog.playlist.pick").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(src) = paths.into_iter().next() else { return };
            let pid = id.clone();
            let saved = cx
                .background_executor()
                .spawn(async move {
                    let img = image::open(&src).map_err(|e| e.to_string())?;
                    // Square crop from the centre, 512 px.
                    let side = img.width().min(img.height());
                    let img = img.crop_imm((img.width() - side) / 2, (img.height() - side) / 2, side, side).thumbnail(512, 512);
                    let dir = glyph_core::paths::data_dir().join("playlist-covers");
                    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    // New name per change: GPUI caches images by path.
                    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
                    let path = dir.join(format!("{pid}-{stamp}.jpg"));
                    img.to_rgb8().save(&path).map_err(|e| e.to_string())?;
                    Ok::<_, String>(path)
                })
                .await;
            this.update(cx, |s, cx| {
                match saved {
                    Ok(path) => {
                        let old = s.library.playlist(&id).and_then(|p| p.cover.clone());
                        if guard(|| s.library.set_playlist_cover(&id, Some(path.display().to_string()))).is_some() {
                            s.save_library(cx);
                            if let Some(old) = old {
                                let _ = std::fs::remove_file(old);
                            }
                        }
                    }
                    Err(e) => s.toast_info(tf("toast.cover_error", &[("e", &e)])),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn remove_playlist_cover(&mut self, id: &str, cx: &mut Context<Self>) {
        let old = self.library.playlist(id).and_then(|p| p.cover.clone());
        if guard(|| self.library.set_playlist_cover(id, None)).is_some() {
            self.save_library(cx);
            if let Some(old) = old {
                let _ = std::fs::remove_file(old);
            }
        }
        cx.notify();
    }

    pub fn close_name_dialog(&mut self, cx: &mut Context<Self>) {
        if self.name_dialog.take().is_some() {
            cx.notify();
        }
    }

    pub fn submit_name_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.name_dialog.take() else { return };
        let typed = self.playlist_name.read(cx).value.trim().to_string();
        if d.eq_preset {
            let name = glyph_core::eq::clean_name(&typed);
            if !name.is_empty() {
                let gains = eq_gains(&self.config.audio.eq_gains);
                self.add_eq_preset(glyph_core::eq::EqPreset::new(&name, gains), cx);
                self.toast_sub(t("toast.eq_saved"), name);
            }
        } else if let Some(id) = d.rename {
            if !typed.is_empty() && guard(|| self.library.rename_playlist(&id, &typed)).is_some() {
                self.save_library(cx);
            }
        } else {
            let name = if typed.is_empty() {
                tf("library.new_playlist_name", &[("n", &(self.library.playlists.len() + 1))]).to_string()
            } else {
                typed
            };
            match guard(|| self.library.create_playlist(&name).id.clone()) {
                Some(id) => {
                    self.save_library(cx);
                    match d.tracks.len() {
                        0 => self.open_list(ListRef::User(id), cx),
                        1 => self.add_to_playlist(&id, d.tracks[0].clone(), cx),
                        _ => self.add_many_to_playlist(&id, d.tracks, cx),
                    }
                }
                None => self.toast_info(t("status.library_offline")),
            }
        }
        cx.notify();
    }

    // ------------------------------------------------------------ settings

    // ------------------------------------------------------------ settings modal

    /// 0..1 how far the settings modal is shown (eased open / close).
    pub fn settings_k(&self) -> f32 {
        if self.settings_open {
            let k = (self.settings_at.elapsed().as_secs_f32() / 0.18).min(1.0);
            1.0 - (1.0 - k).powi(3)
        } else if let Some(at) = self.settings_closing {
            let k = (at.elapsed().as_secs_f32() / 0.14).min(1.0);
            (1.0 - k).powi(2)
        } else {
            0.0
        }
    }

    pub fn open_settings(&mut self, page: Option<SettingsPage>, cx: &mut Context<Self>) {
        if !self.settings_open {
            self.settings_at = Instant::now();
        }
        self.settings_closing = None;
        self.settings_open = true;
        if let Some(p) = page {
            if p != self.settings_page {
                self.settings_sub = 0;
            }
            self.settings_page = p;
        }
        self.open_select = None;
        if self.settings_page == SettingsPage::Storage {
            self.refresh_storage(cx);
        }
        self.refresh_output_devices(cx);
        cx.notify();
    }

    pub fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_open {
            self.settings_closing = Some(Instant::now());
        }
        self.settings_open = false;
        self.open_select = None;
        let f = self.focus.clone();
        window.focus(&f, cx);
        cx.notify();
    }

    pub fn set_settings_page(&mut self, page: SettingsPage, cx: &mut Context<Self>) {
        self.set_settings_tab(page, 0, cx);
    }

    /// Opens a settings page at one of its sub-tabs.
    pub fn set_settings_tab(&mut self, page: SettingsPage, sub: u8, cx: &mut Context<Self>) {
        if (page, sub) != (self.settings_page, self.settings_sub) {
            self.settings_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        }
        self.settings_page = page;
        self.settings_sub = sub;
        self.open_select = None;
        if page == SettingsPage::Storage {
            self.refresh_storage(cx);
        }
        if page == SettingsPage::Account {
            self.account_page_opened(cx);
        }
        cx.notify();
    }

    pub fn toggle_select(&mut self, id: &'static str, cx: &mut Context<Self>) {
        self.open_select = if self.open_select == Some(id) { None } else { Some(id) };
        cx.notify();
    }

    /// Measures the storage folders off the UI thread.
    pub fn refresh_storage(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let stats = cx.background_executor().spawn(async { glyph_core::storage::stats() }).await;
            this.update(cx, |s, cx| {
                s.storage = Some(stats);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn clear_storage(&mut self, kind: glyph_core::storage::StorageKind, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { glyph_core::storage::clear(kind) }).await;
            this.update(cx, |s, cx| {
                if let Err(e) = res {
                    s.toast_info(format!("{e:#}"));
                }
                if kind == glyph_core::storage::StorageKind::Covers {
                    s.covers.clear();
                    s.ascii_covers.clear();
                }
                if kind == glyph_core::storage::StorageKind::Downloads {
                    if let Some(b) = &s.backend {
                        b.reload_downloads();
                    }
                }
                s.refresh_storage(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Cover cache limit in MB (0 = no limit); applies on the next backend build.
    pub fn set_cache_limit(&mut self, mb: u64, cx: &mut Context<Self>) {
        self.config.storage.cache_limit_mb = mb;
        self.save_config(cx);
        if let Some(b) = self.backend.clone() {
            let cfg = self.config.clone();
            guard(|| b.set_config(cfg));
        }
        cx.notify();
    }

    /// Crossfade between songs, 0 (off) to 20 s.
    pub fn set_crossfade(&mut self, secs: u32, cx: &mut Context<Self>) {
        if self.config.audio.crossfade_s == secs.min(20) {
            return;
        }
        self.config.audio.crossfade_s = secs.min(20);
        if let Some(p) = &self.player {
            p.set_crossfade(Duration::from_secs(self.config.audio.crossfade_s as u64));
        }
        self.save_config(cx);
        cx.notify();
    }

    pub fn set_volume_level(&mut self, l: glyph_core::config::VolumeLevel, cx: &mut Context<Self>) {
        self.config.audio.level = l;
        self.open_select = None;
        if let Some(p) = &self.player {
            p.set_loudness(level_of(l), self.config.audio.normalize);
        }
        self.save_config(cx);
        cx.notify();
    }

    /// Equalizer changes apply live; the config is written when asked
    /// (`save`), so dragging a band does not write the file on every move.
    pub fn set_eq(&mut self, enabled: bool, preset: &str, gains: [f32; 10], save: bool, cx: &mut Context<Self>) {
        self.config.audio.eq_enabled = enabled;
        self.config.audio.eq_preset = preset.to_string();
        self.config.audio.eq_gains = gains.to_vec();
        self.open_select = None;
        if let Some(p) = &self.player {
            p.set_equalizer(enabled, gains);
        }
        if save {
            self.save_config(cx);
        }
        cx.notify();
    }

    /// The curve as it is now, named after its preset (for sharing).
    fn current_eq_preset(&self) -> glyph_core::eq::EqPreset {
        let name = crate::ui::settings::eq_preset_label(&self.config.audio.eq_preset);
        glyph_core::eq::EqPreset::new(&name, eq_gains(&self.config.audio.eq_gains))
    }

    /// Asks for a name and saves the current curve as an own preset.
    pub fn open_eq_save_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.config.audio.eq_preset.strip_prefix("user:").unwrap_or_default().to_string();
        self.playlist_name.update(cx, |i, cx| i.set_value(current, cx));
        self.ctx_menu = None;
        self.open_select = None;
        self.name_dialog = Some(NameDialog { tracks: Vec::new(), rename: None, eq_preset: true });
        let f = self.playlist_name.read(cx).focus.clone();
        window.focus(&f, cx);
        cx.notify();
    }

    /// Adds an own preset (one with the same name is replaced) and selects it.
    pub fn add_eq_preset(&mut self, p: glyph_core::eq::EqPreset, cx: &mut Context<Self>) {
        let gains = p.gains();
        let list = &mut self.config.audio.eq_user_presets;
        match list.iter_mut().find(|e| e.name.eq_ignore_ascii_case(&p.name)) {
            Some(e) => *e = p.clone(),
            None => list.push(p.clone()),
        }
        self.set_eq(true, &format!("user:{}", p.name), gains, true, cx);
    }

    /// Removes an own preset; the curve stays as it is.
    pub fn delete_eq_preset(&mut self, name: &str, cx: &mut Context<Self>) {
        self.config.audio.eq_user_presets.retain(|p| p.name != name);
        if self.config.audio.eq_preset.strip_prefix("user:") == Some(name) {
            self.config.audio.eq_preset = "custom".into();
        }
        self.open_select = None;
        self.save_config(cx);
        self.toast_sub(t("toast.eq_deleted"), name.to_string());
        cx.notify();
    }

    /// Copies a share code for the current curve.
    pub fn copy_eq_code(&mut self, cx: &mut Context<Self>) {
        let p = self.current_eq_preset();
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(glyph_core::eq::encode(&p)));
        self.open_select = None;
        self.toast_sub(t("toast.eq_copied"), p.name);
        cx.notify();
    }

    /// Imports an equalizer share code as an own preset; false when the
    /// text is no such code.
    pub fn import_eq_code(&mut self, code: &str, cx: &mut Context<Self>) -> bool {
        if !glyph_core::eq::is_code(code) && !code.trim().starts_with('{') {
            return false;
        }
        match glyph_core::eq::decode(code) {
            Ok(p) => {
                let name = p.name.clone();
                self.add_eq_preset(p, cx);
                self.toast_sub(t("toast.eq_imported"), name);
            }
            Err(e) => self.toast_info(format!("{e:#}")),
        }
        cx.notify();
        true
    }

    /// Imports the equalizer code on the clipboard.
    pub fn paste_eq_code(&mut self, cx: &mut Context<Self>) {
        self.open_select = None;
        let text = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
        if !self.import_eq_code(&text, cx) {
            self.toast_info(t("toast.eq_no_code"));
            cx.notify();
        }
    }

    /// Saves the current curve as a `.glyph-eq` file.
    pub fn export_eq_file(&mut self, cx: &mut Context<Self>) {
        self.open_select = None;
        let p = self.current_eq_preset();
        let file_name = format!("{}.{}", p.name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "-"), glyph_core::eq::FILE_EXT);
        let dir = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_else(glyph_core::paths::data_dir);
        let rx = cx.prompt_for_new_path(&dir, Some(&file_name));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            let body = glyph_core::eq::to_file(&p);
            let saved = cx.background_executor().spawn(async move { std::fs::write(&path, body).map_err(|e| e.to_string()) }).await;
            this.update(cx, |s, cx| {
                match saved {
                    Ok(()) => s.toast_sub(t("toast.eq_exported"), p.name.clone()),
                    Err(e) => s.toast_info(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Imports a `.glyph-eq` file as an own preset.
    pub fn import_eq_file(&mut self, cx: &mut Context<Self>) {
        self.open_select = None;
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t("sp.eq.import_file").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let text = cx.background_executor().spawn(async move { std::fs::read_to_string(path) }).await;
            this.update(cx, |s, cx| {
                match text {
                    Ok(text) if s.import_eq_code(&text, cx) => {}
                    Ok(_) => s.toast_info(t("toast.eq_no_code")),
                    Err(e) => s.toast_info(e.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Re-reads the list of audio outputs (off the UI thread).
    pub fn refresh_output_devices(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let list = cx.background_executor().spawn(async move { glyph_audio::PlayerController::output_devices() }).await;
            this.update(cx, |s, cx| {
                s.output_devices = list;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn set_output_device(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        self.config.audio.output_device = name.clone();
        self.open_select = None;
        if let Some(p) = &self.player {
            p.set_output_device(name);
        }
        self.save_config(cx);
        cx.notify();
    }

    pub fn toggle_normalize(&mut self, cx: &mut Context<Self>) {
        self.config.audio.normalize = !self.config.audio.normalize;
        if let Some(p) = &self.player {
            p.set_loudness(level_of(self.config.audio.level), self.config.audio.normalize);
        }
        self.save_config(cx);
        cx.notify();
    }

    pub fn set_audio_quality(&mut self, q: glyph_core::config::AudioQuality, cx: &mut Context<Self>) {
        self.config.audio.quality = q;
        self.open_select = None;
        self.save_config(cx);
        if let Some(b) = self.backend.clone() {
            let cfg = self.config.clone();
            guard(|| b.set_config(cfg));
        }
        cx.notify();
    }

    /// Quality of new downloads (songs already saved stay as they are).
    pub fn set_download_quality(&mut self, q: glyph_core::config::AudioQuality, cx: &mut Context<Self>) {
        self.config.audio.download_quality = q;
        self.open_select = None;
        self.save_config(cx);
        if let Some(b) = self.backend.clone() {
            let cfg = self.config.clone();
            guard(|| b.set_config(cfg));
        }
        cx.notify();
    }

    /// Version, OS and setup summary for bug reports (no keys, no personal data).
    pub fn diagnostics(&self) -> String {
        let mode = match &self.backend_state {
            BackendState::Ready(BackendMode::Server { url, server_version }) => format!("server {url} v{server_version}"),
            BackendState::Ready(m) => format!("{m:?}"),
            other => format!("{other:?}"),
        };
        format!(
            "Glyph {}
OS: {} {}
Target: {}
Protocol: v{}
Backend: {}
Providers: {:?}
Language: {}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            glyph_core::updater::current_target(),
            glyph_protocol::PROTOCOL_VERSION,
            mode,
            self.config.providers.enabled,
            self.lang.code(),
        )
    }

    pub fn set_intensity(&mut self, v: f32, cx: &mut Context<Self>) {
        self.config.visuals.intensity = v.clamp(0.0, 1.0);
        self.glow.update(cx, |g, _| g.set_intensity(v));
        self.save_config(cx);
        cx.notify();
    }

    pub fn toggle_provider(&mut self, p: ProviderId, cx: &mut Context<Self>) {
        let en = &mut self.config.providers.enabled;
        if let Some(i) = en.iter().position(|x| *x == p) {
            en.remove(i);
        } else {
            en.push(p);
        }
        self.save_config(cx);
        cx.notify();
    }

    /// Drag-and-drop reorder of the source priority list.
    pub fn reorder_priority(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let pr = &mut self.config.providers.priority;
        if from == to || from >= pr.len() || to >= pr.len() {
            return;
        }
        let p = pr.remove(from);
        pr.insert(to, p);
        let p = pr.clone();
        if let Some(b) = &self.backend {
            guard(|| b.set_priority(p));
        }
        self.save_config(cx);
        cx.notify();
    }

    pub fn move_priority(&mut self, i: usize, up: bool, cx: &mut Context<Self>) {
        let pr = &mut self.config.providers.priority;
        let j = if up { i.checked_sub(1) } else { Some(i + 1).filter(|j| *j < pr.len()) };
        if let Some(j) = j {
            pr.swap(i, j);
            let p = pr.clone();
            if let Some(b) = &self.backend {
                guard(|| b.set_priority(p));
            }
            self.save_config(cx);
        }
        cx.notify();
    }

    /// Connection test: builds a throw-away backend for the entered URL and
    /// reports the decided mode.
    pub fn test_connection(&mut self, cx: &mut Context<Self>) {
        let mut cfg = self.config.clone();
        cfg.server_url = self.server_input.read(cx).value.trim().to_string();
        if cfg.effective_server_url().is_empty() {
            self.conn_test = Some(ConnTest::Ok(t("settings.server.direct").to_string()));
            cx.notify();
            return;
        }
        self.conn_test = Some(ConnTest::Running);
        cx.spawn(async move |this, cx| {
            let url = cfg.effective_server_url().to_string();
            let r = cx
                .background_executor()
                .spawn(async move {
                    let b = guard_async(Backend::new(cfg)).await?;
                    guard(|| b.mode()).ok_or_else(|| "mode -> not implemented".to_string())
                })
                .await;
            this.update(cx, |s, cx| {
                s.conn_test = Some(match r {
                    Ok(BackendMode::Server { server_version, .. }) => {
                        ConnTest::Ok(tf("settings.server.ok", &[("url", &url), ("v", &server_version)]).to_string())
                    }
                    Ok(BackendMode::Incompatible { protocol_version, .. }) => ConnTest::Failed(
                        tf("status.incompatible", &[("v", &protocol_version)]).to_string(),
                    ),
                    Ok(BackendMode::Direct) => {
                        ConnTest::Failed(tf("settings.server.unreachable", &[("url", &url)]).to_string())
                    }
                    Err(e) => ConnTest::Failed(e),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn apply_server(&mut self, cx: &mut Context<Self>) {
        self.config.server_url = self.server_input.read(cx).value.trim().to_string();
        self.save_config(cx);
        self.retry_backend(cx);
    }

    pub fn check_updates(&mut self, manual: bool, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else {
            self.update.error = Some(t("status.backend_offline").to_string());
            cx.notify();
            return;
        };
        self.update.checking = true;
        self.update.error = None;
        cx.spawn(async move |this, cx| {
            let r = if manual { guard_async(b.check_for_update()).await } else { guard_async(b.check_for_update_auto()).await };
            this.update(cx, |s, cx| {
                s.update.checking = false;
                match r {
                    Ok(UpdateStatus::Skipped) if !manual => {}
                    Ok(st) => {
                        let auto = s.config.updates.auto_install;
                        if let (UpdateStatus::Available(info), true) = (&st, auto) {
                            if info.can_self_install && !manual {
                                let info = info.clone();
                                s.update.status = Some(st);
                                s.install_update(info, cx);
                                return;
                            }
                        }
                        s.update.status = Some(st);
                    }
                    Err(e) => s.update.error = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn available_update(&self) -> Option<&UpdateInfo> {
        match &self.update.status {
            Some(UpdateStatus::Available(i)) => Some(i),
            _ => None,
        }
    }

    pub fn install_update(&mut self, info: UpdateInfo, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        self.update.installing = Some((0, 0));
        let (tx, mut rx) = flume_like::channel::<(u64, u64)>();
        cx.spawn(async move |this, cx| {
            while let Some(p) = rx.recv().await {
                if this
                    .update(cx, |s, cx| {
                        s.update.installing = Some(p);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let r = guard_async(b.install_update(info, tx)).await;
            this.update(cx, |s, cx| {
                s.update.installing = None;
                match r {
                    Ok(()) => s.update.installed = true,
                    Err(e) => s.update.error = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn set_update_channel(&mut self, prerelease: bool, cx: &mut Context<Self>) {
        self.config.updates.channel = if prerelease { "prerelease" } else { "stable" }.into();
        self.save_config(cx);
        cx.notify();
    }

    pub fn toggle_auto_update(&mut self, cx: &mut Context<Self>) {
        self.config.updates.auto_install = !self.config.updates.auto_install;
        self.save_config(cx);
        cx.notify();
    }

    pub fn restart(&mut self, cx: &mut App) {
        cx.restart();
    }
}

impl Glyph {
    pub fn set_language(&mut self, l: crate::i18n::Lang, cx: &mut Context<Self>) {
        crate::i18n::set_lang(l);
        self.lang = l;
        crate::config_ext::set_language(&mut self.config, l);
        self.save_config(cx);
        cx.notify();
    }

    /// YouTube / SoundCloud as fallback (official, visible embed player).
    pub fn embed_fallback(&self) -> bool {
        let en = &self.config.providers.enabled;
        en.contains(&ProviderId::YouTube) || en.contains(&ProviderId::SoundCloud)
    }

    pub fn toggle_embed_fallback(&mut self, cx: &mut Context<Self>) {
        let on = !self.embed_fallback();
        let en = &mut self.config.providers.enabled;
        en.retain(|p| !matches!(p, ProviderId::YouTube | ProviderId::SoundCloud));
        if on {
            en.push(ProviderId::SoundCloud);
            en.push(ProviderId::YouTube);
        }
        self.save_config(cx);
        cx.notify();
    }

    pub fn embed_playing(&self) -> bool {
        self.player_state
            .as_ref()
            .is_some_and(|s| s.backend == Some(glyph_audio::BackendKind::Embed))
    }

    /// Cached cover path for a track; kicks off a fetch the first time.
    pub fn cover_path(&mut self, t: &Track, cx: &mut Context<Self>) -> Option<PathBuf> {
        let key = track_key(t);
        if let Some(v) = self.covers.get(&key) {
            return v.clone();
        }
        // Not cached as a miss before the backend exists: the home view renders
        // before it is ready, and those covers must still load afterwards.
        let b = self.backend.clone()?;
        self.covers.insert(key.clone(), None);
        let t = t.clone();
        self.covers_loading.insert(key.clone());
        cx.spawn(async move |this, cx| {
            let r = guard_async(b.cover(&t)).await;
            this.update(cx, |s, cx| {
                s.covers_loading.remove(&key);
                if let Ok(Some(p)) = r {
                    s.covers.insert(key, Some(p));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        None
    }

    /// The ASCII picture for this cover size is rendered already.
    pub fn ascii_ready(&self, t: &Track, size: f32) -> bool {
        self.ascii_covers.get(&(track_key(t), (size * 2.0) as u32)).is_some_and(|v| v.is_some())
    }

    /// The cover at `path` as an ASCII picture of `px` pixels (rendered once
    /// on the background executor, then cached).
    pub fn ascii_cover(&mut self, t: &Track, path: &std::path::Path, px: u32, cx: &mut Context<Self>) -> Option<Arc<gpui::RenderImage>> {
        let key = (track_key(t), px);
        if let Some(v) = self.ascii_covers.get(&key) {
            return v.clone();
        }
        self.ascii_covers.insert(key.clone(), None);
        let path = path.to_path_buf();
        cx.spawn(async move |this, cx| {
            let img = cx
                .background_executor()
                .spawn(async move { crate::visuals::ascii::cover_raster(&path, px) })
                .await;
            this.update(cx, |s, cx| {
                if let Some(img) = img {
                    s.ascii_covers.insert(key, Some(Arc::new(gpui::RenderImage::new(vec![image::Frame::new(img)]))));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        None
    }

    /// `path` as the artist hero (ASCII raster, or the faded photo when ASCII
    /// covers are off), computed on a background thread.
    pub fn hero_image(&mut self, path: &std::path::Path, cx: &mut Context<Self>) -> Option<Arc<gpui::RenderImage>> {
        let ascii = self.config.visuals.ascii_covers;
        let key = (path.to_path_buf(), ascii);
        if let Some(v) = self.heroes.get(&key) {
            return v.clone();
        }
        self.heroes.insert(key.clone(), None);
        cx.spawn(async move |this, cx| {
            let src = key.0.clone();
            let img = cx
                .background_executor()
                .spawn(async move {
                    if ascii {
                        crate::visuals::ascii::raster(&src, 1680, 460, 0.3)
                    } else {
                        crate::visuals::ascii::photo_fade(&src, 1680, 460, 0.3)
                    }
                })
                .await;
            this.update(cx, |s, cx| {
                if let Some(img) = img {
                    s.heroes.insert(key, Some(Arc::new(gpui::RenderImage::new(vec![image::Frame::new(img)]))));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        None
    }

    pub fn cover_loading(&self, t: &Track) -> bool {
        self.covers_loading.contains(&track_key(t))
    }
}

fn raw_handle(window: &Window) -> Option<raw_window_handle::RawWindowHandle> {
    use raw_window_handle::HasWindowHandle;
    HasWindowHandle::window_handle(window).ok().map(|h| h.as_raw())
}

/// Raw main-window handle for souvlaki on Windows.
fn window_hwnd(window: &Window) -> Option<usize> {
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
