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
}

/// "New playlist" / "Rename" dialog.
#[derive(Debug, Clone)]
pub struct NameDialog {
    /// Songs to add to the new playlist.
    pub tracks: Vec<Track>,
    /// Playlist id when renaming.
    pub rename: Option<String>,
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
    Storage,
    About,
    Support,
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
    pub name_dialog: Option<NameDialog>,
    /// Sidebar icon under the mouse and since when (hover animation).
    pub nav_hover: Option<(View, Instant)>,
    /// Fills the open album with its other songs (search in the background).
    album_task: Option<Task<()>>,
    /// Startup animation while it runs.
    pub splash: Option<crate::ui::splash::Splash>,
    /// Icon the mouse just left: (view, when, hover seconds at leave) for the ease-back.
    pub nav_leave: Option<(View, Instant, f32)>,
    /// Sidebar list tile under the mouse (tooltip).
    pub side_hover: Option<(usize, Instant)>,
    pub playlist_name: Entity<TextInput>,
    pub glow: Entity<GlowView>,
    pub search_input: Entity<TextInput>,
    pub server_input: Entity<TextInput>,
    pub key_inputs: Vec<(&'static str, Entity<TextInput>)>,

    pub config: Config,
    pub backend: Option<Arc<Backend>>,
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
    pub resolving: bool,
    pub palette: Option<Palette>,
    pub cover: Option<CoverArt>,

    pub search: SearchState,
    pub lyrics: LyricsState,
    pub update: UpdateState,
    pub conn_test: Option<ConnTest>,

    pub show_queue: bool,
    pub show_lyrics: bool,
    /// Now Playing shows only the lyrics (global player bar stays visible).
    pub lyrics_only: bool,
    /// Slide state of the player bar.
    pub bar_slide: Slide,
    pub radio: bool,
    pub mini_on_top: bool,
    /// Settings modal.
    pub settings_open: bool,
    pub settings_page: SettingsPage,
    pub settings_search: Entity<TextInput>,
    /// Which dropdown in the settings modal is open.
    pub open_select: Option<&'static str>,
    /// "Advanced" sections in settings are expanded.
    pub settings_advanced: bool,
    /// Storage breakdown for "Cache & Storage" (None = not measured yet).
    pub storage: Option<glyph_core::storage::StorageStats>,
    pub toast: Option<(String, Instant)>,
    pub lang: crate::i18n::Lang,
    /// Eased text ink per background zone (0 light .. 1 dark).
    pub ink: [f32; 3],
    /// Cover cache: track key -> resolved local path (None = no cover / loading).
    pub covers: std::collections::HashMap<String, Option<PathBuf>>,
    /// Covers currently being fetched (drawn as skeletons).
    pub covers_loading: std::collections::HashSet<String>,
    /// Covers rendered as ASCII, per (track, cols, rows); `None` = computing.
    pub ascii_covers: std::collections::HashMap<(String, u32, u32), Option<Vec<crate::visuals::ascii::ColoredRow>>>,
    pub embed: crate::embed::EmbedHost,
    /// Last (track, playing, position/5 s) pushed to the OS media controls.
    media_keys_sent: Option<(Option<(glyph_protocol::ProviderId, String)>, bool, u64)>,
    /// Re-render every tick while something animates (spinners).
    pub started: Instant,
    _subs: Vec<Subscription>,
    _poll: Task<()>,
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
            name_dialog: None,
            nav_hover: None,
            album_task: None,
            // Read straight from disk: the async config load comes too late for the first frame.
            splash: Config::load()
                .map(|c| c.visuals.startup_animation)
                .unwrap_or(true)
                .then(crate::ui::splash::Splash::new),
            nav_leave: None,
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
            resolving: false,
            palette: None,
            cover: None,
            search: SearchState::default(),
            lyrics: LyricsState::default(),
            update: UpdateState::default(),
            conn_test: None,
            show_queue: false,
            show_lyrics: true,
            lyrics_only: false,
            bar_slide: Slide::default(),
            radio: true,
            mini_on_top: true,
            settings_open: false,
            settings_page: SettingsPage::Playback,
            settings_search,
            open_select: None,
            settings_advanced: false,
            storage: None,
            toast: None,
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

    fn apply_config(&mut self, config: Config, cx: &mut Context<Self>) {
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
                // Misses recorded without a backend are not real misses.
                self.covers.retain(|_, v| v.is_some());
                self.ascii_covers.clear();
                if self.config.updates.check_on_start {
                    self.check_updates(false, cx);
                }
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
                PlayerEvent::Error(e) => self.toast(tf("toast.player_error", &[("e", &e)])),
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
        if let Some((_, at)) = &self.toast {
            if at.elapsed() > TOAST_FOR {
                self.toast = None;
                dirty = true;
            }
        }
        if self.is_animating() || crate::ui::widgets::take_skeleton_seen() {
            dirty = true;
        }
        if dirty {
            cx.notify();
        }
    }

    pub fn is_animating(&self) -> bool {
        let toast_fading = self.toast.as_ref().is_some_and(|(_, at)| {
            let e = at.elapsed();
            e < Duration::from_millis(250) || e > TOAST_FOR - Duration::from_millis(400)
        });
        let nav_anim = self.nav_hover.is_some_and(|(v, at)| v == View::NowPlaying || at.elapsed() < Duration::from_millis(700))
            || self.nav_leave.is_some_and(|(_, at, _)| at.elapsed().as_secs_f32() < crate::ui::chrome::NAV_EASE_BACK + 0.05);
        let menu_anim = (self.ctx_menu.is_some() && self.ctx_opened.elapsed() < Duration::from_millis(200)) || self.ctx_closing.is_some();
        let tip_anim = self.side_hover.is_some_and(|(_, at)| at.elapsed() < Duration::from_millis(600));
        toast_fading
            || tip_anim
            || menu_anim
            || self.splash.is_some()
            || nav_anim
            || self.bar_slide.running()
            || self.search.loading
            || self.resolving
            || self.lyrics.loading
            || self.update.checking
            || self.update.installing.is_some()
            || matches!(self.conn_test, Some(ConnTest::Running))
            || self.backend_state == BackendState::Connecting
            || (self.view == View::NowPlaying && self.is_playing())
    }

    // ------------------------------------------------------------ helpers

    pub fn toast(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        tracing::info!("{msg}");
        self.toast = Some((msg, Instant::now()));
    }

    fn with_player(&mut self, f: impl FnOnce(&PlayerController)) {
        if let Some(p) = &self.player {
            if guard(|| f(p)).is_none() {
                self.toast(t("toast.player_not_ready"));
            }
        } else {
            self.toast(t("status.player_offline"));
        }
    }

    pub fn is_playing(&self) -> bool {
        self.player_state.as_ref().is_some_and(|s| s.status == Status::Playing)
    }

    pub fn position(&self) -> Duration {
        self.player_state.as_ref().map(|s| s.position).unwrap_or_default()
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
    }

    pub fn save_config(&mut self, cx: &mut Context<Self>) {
        let cfg = self.config.clone();
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move { guard_res(|| cfg.save()) })
                .await;
            this.update(cx, |s, cx| {
                match r {
                    Ok(()) => s.toast(t("toast.config_saved")),
                    Err(e) => s.toast(tf("toast.config_error", &[("e", &e)])),
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
        self.search.query = q.clone();
        self.search.seq += 1;
        let seq = self.search.seq;
        if self.view != View::Search && !q.trim().is_empty() {
            self.view = View::Search;
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
                    guard(|| b.search(q.trim())).ok_or(Some(t("error.not_ready").to_string()))
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
                                for t in tracks {
                                    if s.is_hidden_artist(&t.artist) {
                                        continue;
                                    }
                                    if s.search.seen.insert((t.source, t.id.clone())) {
                                        s.search.results.push(t);
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
        let q = self.search.query.trim().to_lowercase();
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
        let q = self.search.query.trim().to_lowercase();
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
        let key = track_key(&track);
        self.current = Some(track.clone());
        self.current_source = Some(track.source);
        self.current_preview = false;
        self.resolving = true;
        if self.library_ok && guard(|| self.library.push_recent(track.clone())).is_some() {
            self.save_library(cx);
        }
        let seed = format!("{}{}", track.artist, track.title);
        let palette = Palette::from_seed(&seed);
        self.palette = Some(palette);
        self.glow.update(cx, |g, _| {
            g.set_palette(Some(palette));
            g.set_preset(Preset::from_seed(&seed));
        });
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
            self.toast(t("status.backend_offline"));
            return;
        };

        // Resolve → load → play.
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
                    }
                    Ok(None) => s.toast(crate::i18n::t("toast.no_source")),
                    Err(e) => s.toast(tf("toast.resolve_error", &[("e", &e)])),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();

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
        self.with_player(|p| p.set_volume(v.clamp(0.0, 1.0)));
    }

    pub fn next(&mut self, auto: bool, cx: &mut Context<Self>) {
        match guard(|| if auto { self.queue.next().cloned() } else { self.queue.skip_next().cloned() }) {
            Some(Some(t)) => self.play_track(t, cx),
            Some(None) => {
                if self.radio {
                    self.start_radio(cx);
                } else if auto {
                    self.toast(t("toast.queue_end"));
                }
            }
            None => {
                self.queue_ok = false;
                self.toast("[ QUEUE OFFLINE ]");
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

    fn preload_next(&mut self, cx: &mut Context<Self>) {
        let Some(Some(next)) = guard(|| self.queue.peek_next().cloned()) else { return };
        let Some(b) = self.backend.clone() else { return };
        cx.spawn(async move |this, cx| {
            if let Ok(Some(res)) = guard_async(b.resolve(&next)).await {
                this.update(cx, |s, _| s.with_player(|p| p.preload_next(res.track, res.source)))
                    .ok();
            }
        })
        .detach();
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
                        s.toast(t("toast.radio_empty"));
                    } else {
                        s.play_list(tracks, 0, cx)
                    }
                }
                Ok(_) => s.toast(t("toast.radio_empty")),
                Err(e) => s.toast(tf("toast.radio_error", &[("e", &e)])),
            })
            .ok();
        })
        .detach();
    }

    pub fn toggle_shuffle(&mut self, cx: &mut Context<Self>) {
        let on = !self.shuffle();
        guard(|| self.queue.set_shuffle(on));
        cx.notify();
    }

    pub fn cycle_repeat(&mut self, cx: &mut Context<Self>) {
        let next = match self.repeat() {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        };
        guard(|| self.queue.set_repeat(next));
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
            Some(()) => self.toast(tf(if next { "toast.play_next" } else { "toast.queued" }, &[("title", &title)])),
            None => self.toast("[ QUEUE OFFLINE ]"),
        }
        cx.notify();
    }

    pub fn queue_remove(&mut self, i: usize, cx: &mut Context<Self>) {
        guard(|| self.queue.remove(i));
        cx.notify();
    }

    pub fn queue_move(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        guard(|| self.queue.move_item(from, to));
        cx.notify();
    }

    pub fn toggle_like(&mut self, t: &Track, cx: &mut Context<Self>) {
        if !self.library_ok {
            self.toast(crate::i18n::t("status.library_offline"));
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
        let mut added = 0;
        let mut name = String::new();
        if let Some(p) = self.library.playlists.iter_mut().find(|p| p.id == id) {
            name = p.name.clone();
            for t in tracks {
                if !p.tracks.iter().any(|x| x.id == t.id && x.source == t.source) {
                    p.tracks.push(t);
                    added += 1;
                }
            }
        }
        if added > 0 {
            self.save_library(cx);
            self.toast(tf("toast.added_n_to_playlist", &[("n", &added), ("name", &name)]));
        }
        cx.notify();
    }

    pub fn add_to_playlist(&mut self, id: &str, t: Track, cx: &mut Context<Self>) {
        if let Some(p) = self.library.playlists.iter_mut().find(|p| p.id == id) {
            if !p.tracks.iter().any(|x| x.id == t.id && x.source == t.source) {
                p.tracks.push(t);
                let name = p.name.clone();
                self.save_library(cx);
                self.toast(tf("toast.added_to_playlist", &[("name", &name)]));
            }
        }
        cx.notify();
    }

    pub fn share(&mut self, t: &Track, cx: &mut Context<Self>) {
        match &t.permalink {
            Some(url) => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(url.clone()));
                self.toast(tf("toast.link_copied", &[("source", &t.source.display_name())]));
            }
            None => self.toast(crate::i18n::t("toast.no_link")),
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
        }
    }

    pub fn is_following(&self, name: &str) -> bool {
        self.library_ok && self.library.is_following(name)
    }

    pub fn toggle_follow(&mut self, name: &str, track: Option<Track>, cx: &mut Context<Self>) {
        if !self.library_ok {
            self.toast(t("status.library_offline"));
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
            self.toast(t("status.library_offline"));
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
                show_all: false,
                about_open: false,
                _tasks: vec![songs, info],
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
                self.toast(t("toast.link_copied_plain"));
            }
            None => self.toast(t("toast.no_link")),
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
        }
        cx.notify();
    }

    /// Opens the name dialog for a new playlist (with `track` added) or for
    /// renaming playlist `rename`.
    pub fn open_name_dialog(&mut self, tracks: Vec<Track>, rename: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let current = rename.as_ref().and_then(|id| self.library.playlist(id)).map(|p| p.name.clone()).unwrap_or_default();
        self.playlist_name.update(cx, |i, cx| i.set_value(current, cx));
        self.ctx_menu = None;
        self.name_dialog = Some(NameDialog { tracks, rename });
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
                    Err(e) => s.toast(tf("toast.cover_error", &[("e", &e)])),
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
        if let Some(id) = d.rename {
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
                None => self.toast(t("status.library_offline")),
            }
        }
        cx.notify();
    }

    // ------------------------------------------------------------ settings

    // ------------------------------------------------------------ settings modal

    pub fn open_settings(&mut self, page: Option<SettingsPage>, cx: &mut Context<Self>) {
        self.settings_open = true;
        if let Some(p) = page {
            self.settings_page = p;
        }
        self.open_select = None;
        if self.settings_page == SettingsPage::Storage {
            self.refresh_storage(cx);
        }
        cx.notify();
    }

    pub fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = false;
        self.open_select = None;
        let f = self.focus.clone();
        window.focus(&f, cx);
        cx.notify();
    }

    pub fn set_settings_page(&mut self, page: SettingsPage, cx: &mut Context<Self>) {
        self.settings_page = page;
        self.open_select = None;
        if page == SettingsPage::Storage {
            self.refresh_storage(cx);
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
                    s.toast(format!("{e:#}"));
                }
                if kind == glyph_core::storage::StorageKind::Covers {
                    s.covers.clear();
                    s.ascii_covers.clear();
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

    /// The cover at `path` as ASCII rows; computed on the background executor.
    pub fn ascii_cover(&mut self, t: &Track, path: &std::path::Path, cols: u32, rows: u32, cx: &mut Context<Self>) -> Option<Vec<crate::visuals::ascii::ColoredRow>> {
        let key = (track_key(t), cols, rows);
        if let Some(v) = self.ascii_covers.get(&key) {
            return v.clone();
        }
        self.ascii_covers.insert(key.clone(), None);
        let path = path.to_path_buf();
        cx.spawn(async move |this, cx| {
            let rows_ = cx
                .background_executor()
                .spawn(async move { crate::visuals::ascii::from_image_colored(&path, cols, rows) })
                .await;
            this.update(cx, |s, cx| {
                // Undecodable image: empty rows, the caller shows the image.
                s.ascii_covers.insert(key, Some(rows_.unwrap_or_default()));
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
