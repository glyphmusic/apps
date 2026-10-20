//! Embed backend: the official YouTube IFrame Player / SoundCloud Widget in a
//! small, always visible player. We never touch the audio stream itself.
//!
//! # Design (event-loop constraint)
//!
//! The main thread belongs to GPUI, so a `tao` event loop cannot run there.
//! Two transports sit behind one [`Transport`] trait; the page
//! (`assets/player.html`) and the control protocol are identical for both:
//!
//! * **Webview window** (Windows, Linux): `tao` + `wry` on a dedicated thread
//!   (`EventLoopBuilderExt*::with_any_thread(true)`), frameless 320x180
//!   window with the ASCII label `[ NOW PLAYING · YT ]`, optional
//!   always-on-top. Commands go through `evaluate_script`, events come back
//!   through the wry IPC handler. On Linux this needs webkit2gtk and GTK and
//!   works because GPUI does not use GTK; on Windows it uses WebView2.
//! * **Browser tab** (macOS, and automatic fallback if the webview cannot be
//!   created, e.g. WebView2 runtime / webkit2gtk missing): AppKit insists on
//!   the main thread for `NSWindow`/`WKWebView`, which GPUI owns, so there we
//!   open the very same player page in the user's default browser (visible,
//!   as the terms of service require). Control and events flow through the
//!   local page server: commands are long-polled by the page and `eval`ed,
//!   events are POSTed back. A future macOS-native child webview can be
//!   added as another [`Transport`] without touching the rest.
//!
//! Both transports load the page from a tiny loopback HTTP server
//! ([`server::LocalServer`], `127.0.0.1`, random token in the path) instead of
//! `about:blank`/`data:` URLs, because YouTube refuses to embed into pages
//! without a real http(s) origin (error 153).
//!
//! Control is plain JS strings produced by the pure `script_*` functions
//! below (unit tested); page events are parsed by [`parse_message`].

pub mod host;
pub mod server;
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub mod webview;

use crate::MiniPlayerOptions;
use glyph_protocol::ProviderId;
use std::sync::Arc;
use std::time::Duration;

pub const PLAYER_HTML: &str = include_str!("../../assets/player.html");
pub const MINI_SIZE: (f64, f64) = (320.0, 180.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedPlayState {
    Unstarted,
    Cued,
    Playing,
    Paused,
    Buffering,
    Ended,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EmbedEvent {
    /// The player page finished loading and `window.glyph` exists.
    PageReady,
    /// The YouTube/SoundCloud player object is ready.
    Ready,
    State(EmbedPlayState),
    Time { pos: Duration, dur: Option<Duration> },
    Error(String),
    /// The user closed the mini-player window.
    Closed,
}

/// `(session, event)` callback; sessions let us ignore events of an old video.
pub type EventSink = Arc<dyn Fn(u64, EmbedEvent) + Send + Sync>;

/// What an app-owned webview should do with the player page.
#[derive(Debug, Clone, PartialEq)]
pub enum PageRequest {
    /// Load this loopback URL (same page and long-poll protocol as the browser tab).
    Open(String),
    Visible(bool),
}

pub type PageOpener = Arc<dyn Fn(PageRequest) + Send + Sync>;

static PAGE_OPENER: std::sync::RwLock<Option<PageOpener>> = std::sync::RwLock::new(None);

/// Mobile: the app shell shows the player page in its own WebView instead of
/// the browser. The desktop app never sets this, so its behaviour is unchanged.
pub fn set_page_opener(opener: PageOpener) {
    *PAGE_OPENER.write().unwrap() = Some(opener);
}

fn page_opener() -> Option<PageOpener> {
    PAGE_OPENER.read().unwrap().clone()
}

// ------------------------------------------------------------------ pure parts

/// `"youtube"` / `"soundcloud"`, or `None` when the provider has no embed.
pub fn provider_key(p: ProviderId) -> Option<&'static str> {
    match p {
        ProviderId::YouTube => Some("youtube"),
        ProviderId::SoundCloud => Some("soundcloud"),
        _ => None,
    }
}

/// `[ NOW PLAYING · YT ]`
pub fn mini_label(p: ProviderId) -> String {
    format!("[ NOW PLAYING \u{b7} {} ]", p.badge())
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Accept a bare id or a URL and return what the player API wants:
/// YouTube -> 11 char video id, SoundCloud -> track/playlist URL.
pub fn normalize_embed_id(provider: ProviderId, raw: &str) -> String {
    let raw = raw.trim();
    match provider {
        ProviderId::YouTube => {
            let is_id = |s: &str| {
                s.len() == 11 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            };
            if is_id(raw) {
                return raw.to_string();
            }
            if let Some(q) = raw.split_once("v=").map(|x| x.1) {
                let id: String = q.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
                if !id.is_empty() {
                    return id;
                }
            }
            for marker in ["youtu.be/", "/embed/", "/shorts/", "/live/"] {
                if let Some(rest) = raw.split_once(marker).map(|x| x.1) {
                    let id: String =
                        rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
                    if !id.is_empty() {
                        return id;
                    }
                }
            }
            raw.to_string()
        }
        ProviderId::SoundCloud => {
            if raw.starts_with("http://") || raw.starts_with("https://") {
                raw.to_string()
            } else if !raw.is_empty() && raw.chars().all(|c| c.is_ascii_digit()) {
                format!("https://api.soundcloud.com/tracks/{raw}")
            } else {
                raw.to_string()
            }
        }
        _ => raw.to_string(),
    }
}

pub fn script_load(provider_key: &str, embed_id: &str, autoplay: bool, volume: f32, session: u64) -> String {
    format!(
        "window.glyph.load({},{},{},{},{});",
        json_str(provider_key),
        json_str(embed_id),
        autoplay,
        volume_percent(volume),
        session
    )
}
pub fn script_play() -> String {
    "window.glyph.play();".into()
}
pub fn script_pause() -> String {
    "window.glyph.pause();".into()
}
pub fn script_seek(pos: Duration) -> String {
    format!("window.glyph.seek({:.3});", pos.as_secs_f64())
}
pub fn script_volume(volume: f32) -> String {
    format!("window.glyph.volume({});", volume_percent(volume))
}
pub fn script_label(label: &str) -> String {
    format!("window.glyph.setLabel({});", json_str(label))
}

pub fn volume_percent(v: f32) -> u32 {
    (v.clamp(0.0, 1.0) * 100.0).round() as u32
}

fn secs(v: &serde_json::Value) -> Option<Duration> {
    let f = v.as_f64()?;
    if f.is_finite() && f >= 0.0 {
        Some(Duration::from_secs_f64(f))
    } else {
        None
    }
}

/// Parse a message posted by the player page: `{"t":"state","v":"playing","s":3}`.
pub fn parse_message(raw: &str) -> Option<(u64, EmbedEvent)> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let session = v.get("s").and_then(|s| s.as_u64()).unwrap_or(0);
    let ev = match v.get("t")?.as_str()? {
        "page" => EmbedEvent::PageReady,
        "ready" => EmbedEvent::Ready,
        "state" => EmbedEvent::State(match v.get("v")?.as_str()? {
            "unstarted" => EmbedPlayState::Unstarted,
            "cued" => EmbedPlayState::Cued,
            "playing" => EmbedPlayState::Playing,
            "paused" => EmbedPlayState::Paused,
            "buffering" => EmbedPlayState::Buffering,
            "ended" => EmbedPlayState::Ended,
            _ => return None,
        }),
        "time" => EmbedEvent::Time {
            pos: secs(v.get("pos")?)?,
            dur: v.get("dur").and_then(secs).filter(|d| !d.is_zero()),
        },
        "error" => EmbedEvent::Error(v.get("msg").and_then(|m| m.as_str()).unwrap_or("embed error").to_string()),
        "closed" => EmbedEvent::Closed,
        _ => return None,
    };
    Some((session, ev))
}

// ------------------------------------------------------------------ transport

/// Moves JS strings into the player page and shows/hides/places the window.
pub trait Transport: Send {
    /// Run a script in the page (queued by the transport).
    fn eval(&self, js: String);
    fn set_visible(&self, visible: bool);
    fn set_options(&self, options: MiniPlayerOptions);
    /// Make sure a page exists (browser transport opens a tab if none polls).
    /// Returns `true` if a fresh page was opened and `PageReady` will follow.
    fn ensure_open(&self) -> bool;
    fn name(&self) -> &'static str;
}

pub struct EmbedBackend {
    transport: Box<dyn Transport>,
    page_ready: bool,
    pending: Vec<String>,
    in_host: bool,
}

impl EmbedBackend {
    /// Whether this backend renders in the main-window child webview.
    pub fn is_host(&self) -> bool {
        self.in_host
    }

    /// `host`: when its child webview is attached, the embed lives inside the
    /// main window instead of a popup/browser.
    pub fn start(sink: EventSink, options: MiniPlayerOptions, host: Option<Arc<host::HostQueue>>) -> anyhow::Result<Self> {
        let srv = server::LocalServer::start(sink.clone())?;
        if let Some(q) = host.filter(|q| q.is_attached()) {
            q.set_sink(sink);
            let t = HostTransport::new(q, srv);
            tracing::info!("embed transport: {}", t.name());
            return Ok(Self { transport: Box::new(t), page_ready: false, pending: Vec::new(), in_host: true });
        }
        #[allow(unused_mut)]
        let mut transport: Option<Box<dyn Transport>> = None;
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        {
            match webview::WebviewTransport::spawn(srv.clone(), sink, options) {
                Ok(t) => transport = Some(Box::new(t)),
                Err(e) => tracing::warn!("mini-player webview unavailable ({e:#}); using the browser"),
            }
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        let _ = (&sink, options);
        let transport = transport.unwrap_or_else(|| Box::new(BrowserTransport::new(srv)));
        tracing::info!("embed transport: {}", transport.name());
        Ok(Self { transport, page_ready: false, pending: Vec::new(), in_host: false })
    }

    pub fn transport_name(&self) -> &'static str {
        self.transport.name()
    }

    fn send(&mut self, js: String) {
        if self.page_ready {
            self.transport.eval(js);
        } else {
            self.pending.push(js);
        }
    }

    pub fn page_ready(&mut self) {
        self.page_ready = true;
        for js in std::mem::take(&mut self.pending) {
            self.transport.eval(js);
        }
    }

    pub fn load(&mut self, provider: ProviderId, embed_id: &str, volume: f32, session: u64) -> Result<(), String> {
        let key = provider_key(provider).ok_or_else(|| format!("{} has no embed player", provider.display_name()))?;
        if self.transport.ensure_open() {
            self.page_ready = false;
            self.pending.clear();
        }
        self.transport.set_visible(true);
        self.send(script_label(&mini_label(provider)));
        self.send(script_load(key, &normalize_embed_id(provider, embed_id), true, volume, session));
        Ok(())
    }

    pub fn play(&mut self) {
        self.send(script_play());
    }
    pub fn pause(&mut self) {
        self.send(script_pause());
    }
    pub fn seek(&mut self, pos: Duration) {
        self.send(script_seek(pos));
    }
    pub fn set_volume(&mut self, v: f32) {
        self.send(script_volume(v));
    }
    pub fn set_options(&mut self, o: MiniPlayerOptions) {
        self.transport.set_options(o);
    }
    /// Pause and hide (used when switching to the native backend / stop).
    pub fn park(&mut self) {
        if self.page_ready {
            self.transport.eval(script_pause());
        }
        self.pending.clear();
        self.transport.set_visible(false);
    }
}

/// Child webview inside the main window; commands are queued for the UI thread.
pub struct HostTransport {
    q: Arc<host::HostQueue>,
    _srv: Arc<server::LocalServer>,
}

impl HostTransport {
    pub fn new(q: Arc<host::HostQueue>, srv: Arc<server::LocalServer>) -> Self {
        q.push(host::HostCmd::Navigate(srv.page_url_host()));
        Self { q, _srv: srv }
    }
}

impl Transport for HostTransport {
    fn eval(&self, js: String) {
        self.q.push(host::HostCmd::Eval(js));
    }
    fn set_visible(&self, visible: bool) {
        self.q.push(host::HostCmd::Parked(!visible));
    }
    fn set_options(&self, _options: MiniPlayerOptions) {}
    fn ensure_open(&self) -> bool {
        false
    }
    fn name(&self) -> &'static str {
        "host-child-webview"
    }
}

/// Opens the player page in the default browser (macOS, fallback), or in the
/// app shell's webview when a [`PageOpener`] is set (mobile).
pub struct BrowserTransport {
    srv: Arc<server::LocalServer>,
}

impl BrowserTransport {
    pub fn new(srv: Arc<server::LocalServer>) -> Self {
        Self { srv }
    }
}

impl Transport for BrowserTransport {
    fn eval(&self, js: String) {
        self.srv.push_script(js);
    }
    fn set_visible(&self, visible: bool) {
        if let Some(open) = page_opener() {
            open(PageRequest::Visible(visible));
        }
    }
    fn set_options(&self, _options: MiniPlayerOptions) {}
    fn ensure_open(&self) -> bool {
        let alive = self.srv.poll_age().is_some_and(|a| a < Duration::from_secs(20));
        if alive {
            return false;
        }
        self.srv.clear_queue();
        let url = self.srv.page_url();
        if let Some(open) = page_opener() {
            open(PageRequest::Open(url));
            return true;
        }
        #[cfg(not(glyph_mobile))]
        if let Err(e) = webbrowser::open(&url) {
            tracing::error!("cannot open the browser: {e}");
        }
        #[cfg(glyph_mobile)]
        tracing::error!("no page opener set; the embed player {url} cannot be shown");
        true
    }
    fn name(&self) -> &'static str {
        if page_opener().is_some() { "app-webview" } else { "browser" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_is_ascii_boxed() {
        assert_eq!(mini_label(ProviderId::YouTube), "[ NOW PLAYING \u{b7} YT ]");
        assert_eq!(mini_label(ProviderId::SoundCloud), "[ NOW PLAYING \u{b7} SC ]");
    }

    #[test]
    fn load_script_escapes_ids() {
        let s = script_load("youtube", "ab\"c');alert(1);//", true, 0.5, 7);
        assert_eq!(s, r#"window.glyph.load("youtube","ab\"c');alert(1);//",true,50,7);"#);
    }

    #[test]
    fn simple_scripts() {
        assert_eq!(script_play(), "window.glyph.play();");
        assert_eq!(script_pause(), "window.glyph.pause();");
        assert_eq!(script_seek(Duration::from_millis(12_345)), "window.glyph.seek(12.345);");
        assert_eq!(script_volume(2.0), "window.glyph.volume(100);");
        assert_eq!(script_volume(-1.0), "window.glyph.volume(0);");
    }

    #[test]
    fn youtube_ids() {
        let y = ProviderId::YouTube;
        assert_eq!(normalize_embed_id(y, "dQw4w9WgXcQ"), "dQw4w9WgXcQ");
        assert_eq!(normalize_embed_id(y, "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=3"), "dQw4w9WgXcQ");
        assert_eq!(normalize_embed_id(y, "https://youtu.be/dQw4w9WgXcQ?si=x"), "dQw4w9WgXcQ");
        assert_eq!(normalize_embed_id(y, "https://www.youtube.com/embed/dQw4w9WgXcQ"), "dQw4w9WgXcQ");
    }

    #[test]
    fn soundcloud_ids() {
        let s = ProviderId::SoundCloud;
        assert_eq!(normalize_embed_id(s, "12345"), "https://api.soundcloud.com/tracks/12345");
        assert_eq!(normalize_embed_id(s, "https://soundcloud.com/a/b"), "https://soundcloud.com/a/b");
    }

    #[test]
    fn provider_keys() {
        assert_eq!(provider_key(ProviderId::YouTube), Some("youtube"));
        assert_eq!(provider_key(ProviderId::SoundCloud), Some("soundcloud"));
        assert_eq!(provider_key(ProviderId::Audius), None);
    }

    #[test]
    fn parse_page_messages() {
        assert_eq!(parse_message(r#"{"t":"page"}"#), Some((0, EmbedEvent::PageReady)));
        assert_eq!(
            parse_message(r#"{"t":"state","v":"playing","s":4}"#),
            Some((4, EmbedEvent::State(EmbedPlayState::Playing)))
        );
        assert_eq!(
            parse_message(r#"{"t":"time","pos":1.5,"dur":200,"s":2}"#),
            Some((2, EmbedEvent::Time { pos: Duration::from_millis(1500), dur: Some(Duration::from_secs(200)) }))
        );
        // unknown duration (0 / NaN-as-null) -> None
        assert_eq!(
            parse_message(r#"{"t":"time","pos":0,"dur":0,"s":2}"#),
            Some((2, EmbedEvent::Time { pos: Duration::ZERO, dur: None }))
        );
        assert_eq!(
            parse_message(r#"{"t":"error","msg":"embedding disabled","s":1}"#),
            Some((1, EmbedEvent::Error("embedding disabled".into())))
        );
        assert_eq!(parse_message(r#"{"t":"state","v":"weird"}"#), None);
        assert_eq!(parse_message("drag"), None);
        assert_eq!(parse_message("{"), None);
    }
}
