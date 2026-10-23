//! Glyph desktop app (GPUI).

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use glyph::{assets, guard, instance, integrate};
use gpui::{
    point, px, size, App, Bounds, TitlebarOptions, WindowBackgroundAppearance,
    WindowBounds, WindowDecorations, WindowOptions,
};
use std::time::Duration;

fn main() {
    let started = std::time::Instant::now();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,glyph=debug".into()),
        )
        .init();
    guard::install_panic_hook();
    // The uninstaller deleting all app data: only clean up, never start.
    #[cfg(windows)]
    if glyph::uninstall::requested() {
        glyph::uninstall::forget_credentials();
        return;
    }
    #[cfg(windows)]
    glyph::playback_qos::configure();
    // Second launch (taskbar jump list, double start): hand over and quit.
    if !instance::claim_or_forward(&instance::request_from_args()) {
        return;
    }
    // Start menu / launcher entry named "Glyph" (no installer does it).
    integrate::register();

    // Read once, overlapping disk/TOML work with platform initialization.
    let config = std::thread::spawn(|| {
        glyph_core::Config::load().unwrap_or_else(|e| {
            tracing::warn!("config: {e}");
            glyph_core::Config::default()
        })
    });

    glyph::i18n::set_lang(glyph::i18n::Lang::system());
    let application = gpui_platform::application().with_assets(assets::Assets);
    // macOS hands `glyph://` links (jam invites, playlists) to the running
    // app as Apple Events, not as arguments.
    application.on_open_urls(|urls| {
        for url in urls {
            glyph::instance::open_url(&url);
        }
    });
    // macOS: a click on the Dock icon opens the window Glyph started without.
    application.on_reopen(|_| glyph::instance::post("tray show"));
    application.run(move |cx: &mut App| {
        tracing::info!(elapsed_ms = started.elapsed().as_millis() as u64, "startup: platform ready");
        let config = config.join().unwrap_or_default();
        glyph::init(cx, &config);
        tracing::info!(elapsed_ms = started.elapsed().as_millis() as u64, "startup: fonts ready");
        // Started with the computer and set to wait in the background: no
        // window until the tray icon (macOS: the Dock icon) is clicked.
        let background = glyph::autostart::launched_by_system() && config.desktop.start_in_tray;
        // Keeps the entry pointing at this binary after an update or a move
        // (debug builds would point it at `target/`).
        if !cfg!(debug_assertions) {
            glyph::autostart::refresh(config.desktop.autostart);
        }

        let bounds = Bounds::centered(None, size(px(1280.0), px(820.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Glyph".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(14.0), px(12.0))),
            }),
            window_decorations: Some(WindowDecorations::Client),
            // Linux draws its own rounded corners → needs a transparent surface.
            window_background: if cfg!(any(target_os = "linux", target_os = "freebsd")) {
                WindowBackgroundAppearance::Transparent
            } else {
                WindowBackgroundAppearance::Opaque
            },
            window_min_size: Some(size(px(900.0), px(600.0))),
            app_id: Some("glyph".into()),
            app_owns_titlebar_drag: true,
            inactive_frame_interval: Some(Duration::from_millis(100)),
            icon: x11_icon(),
            show: !background,
            focus: !background,
            ..Default::default()
        };

        let opened = cx.open_window(options, |window, cx| glyph::root(window, config, cx));
        let main_id = match &opened {
            Ok(h) => Some(h.window_id()),
            Err(e) => {
                tracing::error!("failed to open window: {e:#}");
                cx.quit();
                None
            }
        };
        // Only the main window ends the app (the mini player can close alone).
        cx.on_window_closed(move |cx, id| {
            if Some(id) == main_id || cx.windows().is_empty() {
                glyph::tray::remove();
                cx.quit()
            }
        })
        .detach();
        if background {
            use glyph::i18n::t;
            glyph::tray::show(glyph::tray::Labels {
                open: t("tray.open").to_string(),
                play_pause: t("dock.play_pause").to_string(),
                next: t("dock.next").to_string(),
                previous: t("dock.previous").to_string(),
                quit: t("tray.quit").to_string(),
            });
            // Linux windows cannot start hidden; they start minimized.
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            if let Ok(h) = opened {
                let _ = h.update(cx, |_, window, _| window.minimize_window());
            }
        } else {
            cx.activate(true);
        }
        tracing::info!(elapsed_ms = started.elapsed().as_millis() as u64, "startup: window opened");
    });
}

/// Window icon for X11 (Windows uses the embedded .ico, macOS the bundle).
fn x11_icon() -> Option<std::sync::Arc<image::RgbaImage>> {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        let bytes = include_bytes!("../assets/icons/icon-128.png");
        image::load_from_memory(bytes)
            .ok()
            .map(|i| std::sync::Arc::new(i.to_rgba8()))
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    {
        None
    }
}
