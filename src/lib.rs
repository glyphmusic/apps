//! Glyph app UI (GPUI). One library for every shell: the desktop binary
//! (`main.rs`) and the phone apps (`crates/glyph-android`, later iOS) run the
//! same views, animations and state; phones get the compact layout
//! (`ui::compact`).

pub mod app;
pub mod assets;
pub mod config_ext;
pub mod devices;
pub mod discord;
pub mod embed;
pub mod guard;
pub mod i18n;
pub mod instance;
pub mod integrate;
pub mod mcp;
pub mod mcp_tools;
pub mod mpd;
#[cfg(windows)]
pub mod jumplist;
#[cfg(windows)]
pub mod playback_qos;
pub mod theme;
#[cfg(windows)]
pub mod thumbbar;
pub mod ui;
pub mod visuals;

use gpui::{App, AppContext, Entity, Window};

/// Startup work once the platform runs: fonts, key bindings, language.
pub fn init(cx: &mut App, config: &glyph_core::Config) {
    theme::load_fonts(cx);
    ui::bind_keys(cx);
    i18n::set_lang(config_ext::language(config));
}

/// Root view of the main window, built inside `cx.open_window`.
pub fn root(window: &mut Window, config: glyph_core::Config, cx: &mut App) -> Entity<ui::Shell> {
    let app = cx.new(|cx| app::Glyph::new(window, config, cx));
    let glow = app.read(cx).glow.clone();
    let focus = app.read(cx).focus.clone();
    window.focus(&focus, cx);
    cx.new(|_| ui::Shell { glow, app })
}
