//! The desktop around the window (from the GPUI app's `main.rs`, `boot.rs`,
//! `poll.rs`, `menus.rs`): one Glyph at a time, media keys, the taskbar
//! buttons and jump list (Windows), the tray icon, Discord status, the window
//! title and start with the computer.
//!
//! Everything here runs on the main thread: `tick` is called from the pump
//! through `run_on_main_thread`.

use glyph_app::controller::Core;
use glyph_audio::{MediaKeyEvent, MediaKeys, Status};
use glyph_core::dedupe::track_key;
use glyph_protocol::Track;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

static PENDING_SHOW: AtomicBool = AtomicBool::new(false);
static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();
static BASE_ICON: std::sync::OnceLock<Option<(u32, u32, Vec<u8>)>> = std::sync::OnceLock::new();
use std::time::Duration;
use tauri::Manager;

/// Before Tauri starts. `false`: another Glyph runs and got our request
/// (a jump list click, a second start, a `glyph://` link); quit.
pub fn before_start() -> bool {
    #[cfg(windows)]
    crate::playback_qos::configure();
    if !crate::instance::claim_or_forward(&crate::instance::request_from_args()) {
        return false;
    }
    // Start menu / launcher entry named "Glyph" (no installer does it).
    crate::integrate::register();
    true
}

/// Started with the computer and set to wait in the tray.
pub fn start_hidden(core: &Core) -> bool {
    crate::autostart::launched_by_system() && core.st().config.desktop.start_in_tray
}

#[derive(Default)]
struct Desk {
    media: Option<MediaKeys>,
    media_rx: Option<flume::Receiver<MediaKeyEvent>>,
    media_sent: Option<(Option<String>, bool, u64)>,
    discord: Option<crate::discord::DiscordPresence>,
    discord_id: Option<String>,
    discord_sent: Option<(Option<String>, bool, u64)>,
    title: String,
    dock_keys: Vec<String>,
    tray: bool,
    /// Last accent (`0xrrggbb`) applied to window, tray, and Dock icons.
    theme_accent: Option<u32>,
    /// "Mini player on top" as last handed to the player and the window.
    mini_on_top: Option<bool>,
}

thread_local! {
    static DESK: RefCell<Desk> = RefCell::new(Desk::default());
}

static THEME_ACCENT: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1);

use glyph_app::i18n::tr;

fn hwnd(app: &tauri::AppHandle) -> Option<usize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let w = app.get_webview_window("main")?;
    match w.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get() as usize),
        _ => None,
    }
}

#[cfg(windows)]
fn is_app_origin(uri: &str) -> bool {
    ["http://tauri.localhost", "https://tauri.localhost", "tauri://localhost", "http://localhost:1420"]
        .iter()
        .any(|origin| uri == *origin || uri.strip_prefix(origin).is_some_and(|rest| rest.starts_with('/')))
}

/// Disable DevTools, browser shortcuts, default context menus, zoom hotkeys,
/// status bar, Edge's built-in error page, and auto-allow app-origin
/// browser permissions (e.g. clipboard) on desktop webviews.
pub fn harden_webview(win: &tauri::WebviewWindow) {
    #[cfg(windows)]
    {
        let is_mini = win.label() == "mini";
        let _ = win.with_webview(move |wv| unsafe {
            use webview2_com::Microsoft::Web::WebView2::Win32::{
                COREWEBVIEW2_COLOR, COREWEBVIEW2_PERMISSION_STATE_ALLOW,
                ICoreWebView2Controller2, ICoreWebView2PermissionRequestedEventArgs2,
                ICoreWebView2Settings3,
            };
            use webview2_com::{take_pwstr, PermissionRequestedEventHandler};
            use windows_core::{Interface, PWSTR};

            if is_mini {
                if let Ok(c2) = wv.controller().cast::<ICoreWebView2Controller2>() {
                    let _ = c2.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR { A: 0, R: 0, G: 0, B: 0 });
                }
            }

            let Ok(core) = wv.controller().CoreWebView2() else { return };
            if let Ok(settings) = core.Settings() {
                let _ = settings.SetAreDevToolsEnabled(false);
                let _ = settings.SetAreDefaultContextMenusEnabled(false);
                let _ = settings.SetIsStatusBarEnabled(false);
                let _ = settings.SetIsZoomControlEnabled(false);
                let _ = settings.SetIsBuiltInErrorPageEnabled(false);
                if let Ok(settings3) = settings.cast::<ICoreWebView2Settings3>() {
                    let _ = settings3.SetAreBrowserAcceleratorKeysEnabled(false);
                }
            }
            let mut token = 0i64;
            let _ = core.add_PermissionRequested(
                &PermissionRequestedEventHandler::create(Box::new(|_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let mut uri = PWSTR::null();
                    if args.Uri(&mut uri).is_ok() && is_app_origin(&take_pwstr(uri)) {
                        let _ = args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW);
                        if let Ok(args2) = args.cast::<ICoreWebView2PermissionRequestedEventArgs2>() {
                            let _ = args2.SetHandled(true);
                        }
                    }
                    Ok(())
                })),
                &mut token,
            );
        });
    }
    #[cfg(not(windows))]
    let _ = win;
}

/// Switches all open Tauri WebView2 instances between `LOW` (flush V8 / image
/// caches under RAM pressure) and `NORMAL` memory target level.
pub fn set_webview_low_memory(app: &tauri::AppHandle, low: bool) {
    #[cfg(windows)]
    {
        let h = app.clone();
        let _ = app.run_on_main_thread(move || {
            use webview2_com::Microsoft::Web::WebView2::Win32::{
                ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
            };
            use windows_core::Interface;
            let level = if low {
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
            } else {
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
            };
            for label in ["main", "mini"] {
                let Some(win) = h.get_webview_window(label) else { continue };
                let _ = win.with_webview(move |wv| unsafe {
                    if let Ok(core) = wv.controller().CoreWebView2() {
                        if let Ok(c19) = core.cast::<ICoreWebView2_19>() {
                            let _ = c19.SetMemoryUsageTargetLevel(level);
                        }
                    }
                });
            }
        });
    }
    #[cfg(not(windows))]
    let _ = (app, low);
}

/// Suspends the main WebView2 instance when the window is hidden or minimized.
pub fn suspend_main_webview(app: &tauri::AppHandle) {
    #[cfg(windows)]
    {
        let h = app.clone();
        let _ = app.run_on_main_thread(move || {
            use webview2_com::Microsoft::Web::WebView2::Win32::{
                ICoreWebView2_3, ICoreWebView2_19,
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
            };
            use webview2_com::TrySuspendCompletedHandler;
            use windows_core::Interface;

            let Some(win) = h.get_webview_window("main") else { return };
            let is_minimized = win.is_minimized().unwrap_or(false);
            let is_visible = win.is_visible().unwrap_or(true);
            if !is_minimized && is_visible {
                return;
            }

            let _ = win.with_webview(move |wv| unsafe {
                if let Ok(core) = wv.controller().CoreWebView2() {
                    if let Ok(c19) = core.cast::<ICoreWebView2_19>() {
                        let _ = c19.SetMemoryUsageTargetLevel(COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW);
                    }
                    if let Ok(c3) = core.cast::<ICoreWebView2_3>() {
                        let handler = TrySuspendCompletedHandler::create(Box::new(|res, is_suspended| {
                            tracing::debug!(?res, is_suspended, "ICoreWebView2_3::TrySuspend completed");
                            Ok(())
                        }));
                        let res = c3.TrySuspend(&handler);
                        tracing::debug!(?res, "ICoreWebView2_3::TrySuspend initiated");
                    }
                }
            });
        });
    }
    #[cfg(not(windows))]
    let _ = app;
}

/// Immediately resumes the main WebView2 instance if suspended and restores normal memory level.
pub fn resume_main_webview(app: &tauri::AppHandle) {
    #[cfg(windows)]
    {
        let h = app.clone();
        let _ = app.run_on_main_thread(move || {
            use webview2_com::Microsoft::Web::WebView2::Win32::{
                ICoreWebView2_3, ICoreWebView2_19,
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
            };
            use windows_core::Interface;

            let Some(win) = h.get_webview_window("main") else { return };
            let _ = win.with_webview(move |wv| unsafe {
                if let Ok(core) = wv.controller().CoreWebView2() {
                    if let Ok(c3) = core.cast::<ICoreWebView2_3>() {
                        let res = c3.Resume();
                        tracing::debug!(?res, "ICoreWebView2_3::Resume completed");
                    }
                    if let Ok(c19) = core.cast::<ICoreWebView2_19>() {
                        let _ = c19.SetMemoryUsageTargetLevel(COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL);
                    }
                }
            });
        });
    }
    #[cfg(not(windows))]
    let _ = app;
}

/// Starts window event listening for minimize / focus / visibility dormancy tracking.
pub fn start(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let handle = app.clone();
        win.on_window_event(move |event| {
            match event {
                tauri::WindowEvent::Focused(focused) => {
                    if *focused {
                        resume_main_webview(&handle);
                    }
                    crate::memory_guard::notify_window_state(&handle);
                }
                tauri::WindowEvent::Resized(_) => {
                    crate::memory_guard::notify_window_state(&handle);
                }
                _ => {}
            }
        });
    }
}

/// Window at start-up: shown, or hidden with a tray icon.
pub fn setup(app: &tauri::App, core: &Arc<Core>) {
    let handle = app.handle().clone();
    let _ = APP.set(handle.clone());
    start(&handle);
    // glyph:// links the system hands to the running app (macOS) or at cold start.
    {
        use tauri_plugin_deep_link::DeepLinkExt;
        if let Ok(Some(urls)) = app.deep_link().get_current() {
            for url in urls {
                crate::instance::open_url(url.as_str());
            }
        }
        app.deep_link().on_open_url(|event| {
            for url in event.urls() {
                crate::instance::open_url(url.as_str());
            }
        });
    }
    let hidden = start_hidden(core);
    // Keeps the entry pointing at this binary after an update or a move
    // (debug builds would point it at `target/`).
    if !cfg!(debug_assertions) {
        crate::autostart::refresh(core.st().config.desktop.autostart);
    }
    if app.get_webview_window("main").is_some() {
        if hidden {
            show_tray(&handle, core);
        } else {
            // Wait briefly for the UI to mount its splash / initial view
            // before revealing the window so the bare background never flashes.
            PENDING_SHOW.store(true, Ordering::Release);
            let h_fallback = handle.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(2500));
                if PENDING_SHOW.swap(false, Ordering::AcqRel) {
                    let h = h_fallback.clone();
                    let _ = h_fallback.run_on_main_thread(move || show_window(&h));
                }
            });
        }
    }
    let h = hwnd(&handle);
    DESK.with(|d| {
        let mut d = d.borrow_mut();
        match glyph_app::guard::guard_res(|| MediaKeys::new(h.map(|h| h as *mut std::ffi::c_void))) {
            Ok(mk) => {
                d.media_rx = Some(mk.events());
                d.media = Some(mk);
            }
            Err(e) => tracing::warn!("media keys: {e}"),
        }
    });
    // Like / previous / play-pause / next under the taskbar preview.
    #[cfg(windows)]
    if let Some(h) = h {
        crate::thumbbar::init(h, [tr(core, "ctx.like"), tr(core, "dock.previous"), tr(core, "dock.play_pause"), tr(core, "dock.next")]);
    }
}

/// Shows the window (tray click, second start).
pub fn show_window(app: &tauri::AppHandle) {
    PENDING_SHOW.store(false, Ordering::Release);
    resume_main_webview(app);
    crate::memory_guard::notify_window_state(app);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn base_icon() -> Option<&'static (u32, u32, Vec<u8>)> {
    BASE_ICON
        .get_or_init(|| glyph_app::theme_icon::decode_png(include_bytes!("../../../assets/icons/icon-256.png")))
        .as_ref()
}

fn themed_icon(accent: u32) -> Option<tauri::image::Image<'static>> {
    let (w, h, base) = base_icon()?;
    let rgba = glyph_app::theme_icon::tint_rgba(base, accent);
    Some(tauri::image::Image::new_owned(rgba, *w, *h))
}

/// Applies `accent` (`0xrrggbb`) to the desktop window, mini player, tray, and
/// macOS Dock icons.
pub fn set_theme_icon(accent: u32) {
    let changed = DESK.with(|d| {
        let mut d = d.borrow_mut();
        if d.theme_accent == Some(accent) {
            return false;
        }
        d.theme_accent = Some(accent);
        THEME_ACCENT.store(i64::from(accent), Ordering::Release);
        true
    });
    if !changed {
        return;
    }
    let Some((w, h, base)) = base_icon() else {
        return;
    };
    let rgba = glyph_app::theme_icon::tint_rgba(base, accent);
    if let Some(app) = APP.get() {
        let icon = tauri::image::Image::new_owned(rgba.clone(), *w, *h);
        if let Some(win) = app.get_webview_window("main") {
            let _ = win.set_icon(icon.clone());
        }
        if let Some(win) = app.get_webview_window("mini") {
            let _ = win.set_icon(icon.clone());
        }
        if let Some(tray) = app.tray_by_id("main") {
            let _ = tray.set_icon(Some(icon));
        }
    }
    #[cfg(target_os = "macos")]
    if let Some(png) = glyph_app::theme_icon::encode_png(*w, *h, &rgba) {
        mac_dock::set_icon(&png);
    }
}

/// Called by the Svelte UI once its initial view (or splash) is mounted.
#[tauri::command(rename_all = "snake_case")]
pub fn ui_ready(app: tauri::AppHandle) {
    if PENDING_SHOW.swap(false, Ordering::AcqRel) {
        show_window(&app);
    }
}

fn show_tray(app: &tauri::AppHandle, core: &Arc<Core>) {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    if DESK.with(|d| d.borrow().tray) {
        return;
    }
    let item = |id: &str, key: &str| MenuItem::with_id(app, id, tr(core, key), true, None::<&str>);
    let built = (|| -> tauri::Result<()> {
        let sep = PredefinedMenuItem::separator(app)?;
        let menu = Menu::with_items(
            app,
            &[
                &item("show", "tray.open")?,
                &sep,
                &item("dock 5", "dock.play_pause")?,
                &item("dock 6", "dock.next")?,
                &item("dock 7", "dock.previous")?,
                &PredefinedMenuItem::separator(app)?,
                &item("quit", "tray.quit")?,
            ],
        )?;
        let mut b = TrayIconBuilder::with_id("main").tooltip("Glyph").menu(&menu).show_menu_on_left_click(false);
        let initial_icon = DESK
            .with(|d| d.borrow().theme_accent)
            .and_then(themed_icon)
            .or_else(|| app.default_window_icon().cloned());
        if let Some(icon) = initial_icon {
            b = b.icon(icon);
        }
        b.on_menu_event(|app, ev| match ev.id().as_ref() {
            "show" => show_window(app),
            "quit" => {
                if let Some(core) = app.try_state::<Arc<Core>>() {
                    core.save_all_now();
                }
                app.exit(0);
            }
            other => crate::instance::post(other),
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                show_window(tray.app_handle());
            }
        })
        .build(app)?;
        Ok(())
    })();
    match built {
        Ok(()) => DESK.with(|d| d.borrow_mut().tray = true),
        Err(e) => tracing::warn!("tray: {e}"),
    }
}

/// Jump list / dock entry `n`: 0..=4 recently played, 5 play/pause,
/// 6 next, 7 previous.
pub fn dock_action(core: &Arc<Core>, n: usize) {
    match n {
        0..=4 => core.play_recent_at(n),
        5 => core.toggle_play(),
        6 => core.next(false),
        7 => core.previous(),
        _ => {}
    }
}

fn toggle_like_current(core: &Arc<Core>) {
    let cur = core.st().current.clone();
    if let Some(t) = cur {
        core.st().library.toggle_like(&t);
        core.library_changed();
    }
}

/// Every 100 ms on the main thread.
pub fn tick(core: &Arc<Core>) {
    let Some(app) = APP.get().cloned() else { return };
    // Media keys (SMTC on Windows, MPRIS on Linux, Now Playing on macOS).
    let keys: Vec<MediaKeyEvent> = DESK.with(|d| d.borrow().media_rx.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default());
    for k in keys {
        match k {
            MediaKeyEvent::Play => {
                if core.player_dto().status != "playing" {
                    core.toggle_play();
                }
            }
            MediaKeyEvent::Pause => {
                if core.player_dto().status == "playing" {
                    core.toggle_play();
                }
            }
            MediaKeyEvent::Toggle => core.toggle_play(),
            MediaKeyEvent::Next => core.next(false),
            MediaKeyEvent::Previous => core.previous(),
            MediaKeyEvent::Stop => {
                if let Some(p) = &core.player {
                    p.stop();
                }
            }
            MediaKeyEvent::SeekForward => core.seek(core.player_dto().position + 5.0),
            MediaKeyEvent::SeekBackward => core.seek((core.player_dto().position - 5.0).max(0.0)),
            MediaKeyEvent::Like => toggle_like_current(core),
            MediaKeyEvent::Volume(_) => {}
        }
    }

    // Taskbar buttons under the preview.
    #[cfg(windows)]
    {
        for id in crate::thumbbar::take_clicks() {
            match id {
                crate::thumbbar::LIKE => toggle_like_current(core),
                crate::thumbbar::PREV => core.previous(),
                crate::thumbbar::PLAY => core.toggle_play(),
                crate::thumbbar::NEXT => core.next(false),
                _ => {}
            }
        }
    }

    // Requests from the tray, the jump list or another start of Glyph.
    while let Some(req) = crate::instance::try_recv() {
        let req = req.strip_prefix("tray ").map(str::to_string).unwrap_or(req);
        if req == "show" || req == "activate" {
            show_window(&app);
        } else if req == "quit" {
            core.save_all_now();
            app.exit(0);
        } else if let Some(n) = req.strip_prefix("dock ").and_then(|n| n.parse::<usize>().ok()) {
            dock_action(core, n);
        } else if let Some(code) = req.strip_prefix("jam ") {
            glyph_app::jam::open_link(core, code);
            show_window(&app);
        } else if let Some(id) = req.strip_prefix("plugin ") {
            glyph_app::plugins::open_plugin(core, id);
            show_window(&app);
        } else if let Some(text) = req.strip_prefix("open ").or_else(|| req.strip_prefix("shared ")) {
            core.emit("shared", text.to_string());
            show_window(&app);
        }
    }

    let (current, liked, playing, position, config) = {
        let st = core.st();
        let liked = st.current.as_ref().is_some_and(|c| st.library.is_liked(c));
        let ps = core.player_state();
        // A song being found counts as playing for the OS, unless pause was
        // pressed meanwhile (GPUI poll.rs publish_now_playing).
        let playing = (ps.as_ref().is_some_and(|s| s.status == Status::Playing) && !st.resolving)
            || (st.resolving && !glyph_app::jam::hold_play());
        let pos = ps.map_or(Duration::ZERO, |s| s.position);
        (st.current.clone(), liked, playing, pos, st.config.clone())
    };
    #[cfg(windows)]
    crate::thumbbar::set_state(playing, liked);
    #[cfg(not(windows))]
    let _ = liked;

    let key = |every: u64| (current.as_ref().map(track_key), playing, position.as_secs() / every);
    DESK.with(|d| {
        let mut d = d.borrow_mut();
        // OS media controls: only track, play state and a coarse position.
        let k = key(5);
        if d.media_sent.as_ref() != Some(&k) {
            if let Some(mk) = &d.media {
                glyph_app::guard::guard(|| mk.set_now_playing(current.as_ref(), playing, position));
            }
            d.media_sent = Some(k);
        }
        // Discord status, as Settings > Integrations says.
        let want = config.integrations.discord_presence.then(|| crate::discord::client_id(&config)).flatten();
        if want != d.discord_id {
            d.discord = want.clone().map(crate::discord::DiscordPresence::new);
            d.discord_id = want;
            d.discord_sent = None;
        }
        // 15 s steps keep us far below Discord's rate limit.
        let k = key(15);
        if d.discord.is_some() && d.discord_sent.as_ref() != Some(&k) {
            if let Some(dp) = &d.discord {
                dp.set(current.as_ref(), playing, position);
                // In a jam Discord shows it as a party.
                dp.set_party(glyph_app::jam::discord_party());
            }
            d.discord_sent = Some(k);
        }
        // "Mini player on top": the YouTube embed's mini window and ours.
        let on_top = config.player.mini_on_top;
        if d.mini_on_top != Some(on_top) {
            if let Some(p) = &core.player {
                p.set_mini_player_options(glyph_audio::MiniPlayerOptions { always_on_top: on_top, position: None });
            }
            if let Some(w) = app.get_webview_window("mini") {
                let _ = w.set_always_on_top(on_top);
            }
            d.mini_on_top = Some(on_top);
        }
        // Window title (taskbar, alt-tab).
        let title = current.as_ref().map_or_else(|| "Glyph".to_string(), |t| format!("{} - {}", t.artist, t.title));
        if title != d.title {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_title(&title);
            }
            d.title = title;
        }
    });
    update_jump_list(core);
}

/// Recently played songs + play / next / previous in the taskbar jump list.
fn update_jump_list(core: &Arc<Core>) {
    let recent: Vec<Track> = core.st().library.recent_tracks().take(5).cloned().collect();
    let keys: Vec<String> = recent.iter().map(track_key).collect();
    if DESK.with(|d| d.borrow().dock_keys == keys) {
        return;
    }
    DESK.with(|d| d.borrow_mut().dock_keys = keys);
    #[cfg(windows)]
    {
        use crate::jumplist::{update, Entry};
        let entries = recent
            .iter()
            .enumerate()
            .map(|(i, t)| Entry { label: format!("{} · {}", t.title, t.artist), action: i })
            .collect();
        let tasks = vec![
            Entry { label: tr(core, "dock.play_pause"), action: 5 },
            Entry { label: tr(core, "dock.next"), action: 6 },
            Entry { label: tr(core, "dock.previous"), action: 7 },
        ];
        update(tr(core, "library.recent"), entries, tasks);
    }
    // macOS: the same entries in the Dock icon's menu.
    #[cfg(target_os = "macos")]
    {
        let mut items: Vec<(String, usize)> =
            recent.iter().enumerate().map(|(i, t)| (format!("{} · {}", t.title, t.artist), i)).collect();
        items.push((tr(core, "dock.play_pause"), 5));
        items.push((tr(core, "dock.next"), 6));
        items.push((tr(core, "dock.previous"), 7));
        mac_dock::set(items);
    }
}

/// The Dock menu on macOS. AppKit asks the app delegate for it
/// (`applicationDockMenu:`); Tauri's delegate has no such method, so it is
/// added to the delegate's class. A click posts `dock N` like the jump list.
#[cfg(target_os = "macos")]
mod mac_dock {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Sel};
    use objc2::{define_class, msg_send, sel, AnyThread, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSApplication, NSImage, NSMenu, NSMenuItem};
    use objc2_foundation::{NSData, NSObject, NSString};
    use std::cell::RefCell;

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "GlyphDockTarget"]
        struct Target;

        impl Target {
            #[unsafe(method(dockItem:))]
            fn dock_item(&self, sender: &NSMenuItem) {
                crate::instance::post(&format!("dock {}", sender.tag()));
            }
        }
    );

    thread_local! {
        static MENU: RefCell<Option<Retained<NSMenu>>> = const { RefCell::new(None) };
        static TARGET: RefCell<Option<Retained<Target>>> = const { RefCell::new(None) };
        static HOOKED: RefCell<bool> = const { RefCell::new(false) };
    }

    unsafe extern "C-unwind" fn dock_menu(_this: &AnyObject, _cmd: Sel, _app: &AnyObject) -> *mut NSMenu {
        MENU.with(|m| m.borrow().as_ref().map_or(std::ptr::null_mut(), |m| Retained::as_ptr(m) as *mut NSMenu))
    }

    /// Adds `applicationDockMenu:` to the app delegate's class (once).
    fn hook(mtm: MainThreadMarker) {
        if HOOKED.with(|h| std::mem::replace(&mut *h.borrow_mut(), true)) {
            return;
        }
        let app = NSApplication::sharedApplication(mtm);
        #[allow(unused_unsafe)]
        let Some(delegate) = (unsafe { app.delegate() }) else { return };
        let obj: &AnyObject = unsafe { &*(Retained::as_ptr(&delegate) as *const AnyObject) };
        let cls = obj.class();
        unsafe {
            let imp: objc2::runtime::Imp = std::mem::transmute(
                dock_menu as unsafe extern "C-unwind" fn(&AnyObject, Sel, &AnyObject) -> *mut NSMenu,
            );
            // "@@:@": returns an object, takes self, _cmd and the app.
            let types = c"@@:@";
            objc2::ffi::class_addMethod(cls as *const _ as *mut _, sel!(applicationDockMenu:), imp, types.as_ptr());
        }
    }

    pub fn set(items: Vec<(String, usize)>) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        hook(mtm);
        let target = TARGET.with(|t| {
            t.borrow_mut()
                .get_or_insert_with(|| unsafe { msg_send![Target::alloc(), init] })
                .clone()
        });
        let menu = NSMenu::new(mtm);
        for (label, action) in items {
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(&label),
                    Some(sel!(dockItem:)),
                    &NSString::from_str(""),
                )
            };
            unsafe { item.setTarget(Some(&target)) };
            item.setTag(action as isize);
            menu.addItem(&item);
        }
        MENU.with(|m| *m.borrow_mut() = Some(menu));
    }

    pub fn set_icon(png: &[u8]) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let data = NSData::with_bytes(png);
        let Some(img) = NSImage::initWithData(NSImage::alloc(), &data) else { return };
        let app = NSApplication::sharedApplication(mtm);
        unsafe { app.setApplicationIconImage(Some(&img)) };
    }
}

/// The mini player (from `src/ui/mini.rs`): an always-on-top window of its
/// own that loads the same UI with `#mini`. The bar is 300 x 52; the card
/// keeps its size in `config.player.mini_card_w / _h`.
#[tauri::command(rename_all = "snake_case")]
pub async fn mini_show(app: tauri::AppHandle, core: tauri::State<'_, Arc<Core>>, shown: bool) -> Result<(), String> {
    let existing = app.get_webview_window("mini");
    if !shown {
        if let Some(w) = existing {
            let _ = w.close();
        }
        core.emit("mini", false);
        return Ok(());
    }
    if let Some(w) = existing {
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    let (on_top, large, cw, ch) = {
        let p = &core.st().config.player;
        (p.mini_on_top, p.mini_large, p.mini_card_w as f64, p.mini_card_h as f64)
    };
    let (w, h) = if large { (cw, ch) } else { (300.0, 52.0) };
    let b = tauri::WebviewWindowBuilder::new(&app, "mini", tauri::WebviewUrl::App("index.html#mini".into()))
        .title("Glyph")
        .devtools(false)
        .inner_size(w, h)
        .min_inner_size(240.0, 52.0)
        .decorations(false)
        .always_on_top(on_top)
        .skip_taskbar(true)
        .resizable(true)
        .shadow(false)
        .transparent(true)
        .background_color(tauri::window::Color(0, 0, 0, 0))
        .no_redirection_bitmap(true);
    let win = b.build().map_err(|e| e.to_string())?;
    harden_webview(&win);
    let accent = THEME_ACCENT.load(Ordering::Acquire);
    if accent >= 0 {
        if let Some(icon) = themed_icon(accent as u32) {
            let _ = win.set_icon(icon);
        }
    }
    // Closed by the system (alt-F4): the main window's button follows.
    let c = core.inner().clone();
    win.on_window_event(move |ev| {
        if let tauri::WindowEvent::Destroyed = ev {
            c.emit("mini", false);
        }
    });
    core.emit("mini", true);
    Ok(())
}
