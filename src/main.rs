//! Glyph desktop app (GPUI).

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod assets;
mod config_ext;
mod embed;
mod i18n;
mod guard;
mod theme;
mod ui;
mod visuals;

use gpui::{
    point, px, size, App, AppContext, Bounds, TitlebarOptions, WindowBackgroundAppearance,
    WindowBounds, WindowDecorations, WindowOptions,
};
use std::time::Duration;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,glyph=debug".into()),
        )
        .init();
    guard::install_panic_hook();

    crate::i18n::set_lang(crate::i18n::Lang::system());
    gpui_platform::application().with_assets(assets::Assets).run(|cx: &mut App| {
        theme::load_fonts(cx);
        ui::bind_keys(cx);

        let bounds = Bounds::centered(None, size(px(1280.0), px(820.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("glyph".into()),
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
            let app = cx.new(|cx| app::Glyph::new(window, cx));
            let glow = app.read(cx).glow.clone();
            let focus = app.read(cx).focus.clone();
            window.focus(&focus, cx);
            cx.new(|_| ui::Shell { glow, app })
        });
        if let Err(e) = opened {
            tracing::error!("failed to open window: {e:#}");
            cx.quit();
        }
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        cx.activate(true);
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
