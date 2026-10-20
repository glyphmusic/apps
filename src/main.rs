//! Glyph desktop app (GPUI).

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod assets;
mod config_ext;
mod devices;
mod discord;
mod embed;
mod i18n;
mod instance;
mod integrate;
mod mcp;
mod mcp_tools;
mod mpd;
#[cfg(windows)]
mod jumplist;
#[cfg(windows)]
mod thumbbar;
mod guard;
mod theme;
mod ui;
mod visuals;
#[cfg(windows)]
mod playback_qos;

use gpui::{
    point, px, size, App, AppContext, Bounds, TitlebarOptions, WindowBackgroundAppearance,
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
    #[cfg(windows)]
    playback_qos::configure();
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

    crate::i18n::set_lang(crate::i18n::Lang::system());
    gpui_platform::application().with_assets(assets::Assets).run(move |cx: &mut App| {
        tracing::info!(elapsed_ms = started.elapsed().as_millis() as u64, "startup: platform ready");
        theme::load_fonts(cx);
        tracing::info!(elapsed_ms = started.elapsed().as_millis() as u64, "startup: fonts ready");
        ui::bind_keys(cx);
        let config = config.join().unwrap_or_default();
        crate::i18n::set_lang(crate::config_ext::language(&config));

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
            ..Default::default()
        };

        let opened = cx.open_window(options, |window, cx| {
            let app = cx.new(|cx| app::Glyph::new(window, config, cx));
            let glow = app.read(cx).glow.clone();
            let focus = app.read(cx).focus.clone();
            window.focus(&focus, cx);
            cx.new(|_| ui::Shell { glow, app })
        });
        let main_id = match opened {
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
                cx.quit()
            }
        })
        .detach();
        cx.activate(true);
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
