//! Glyph's visual tokens: deep black, floating translucent panels, hairline
//! borders, no shadows, monospace everywhere.

use gpui::{hsla, App, Hsla, Pixels, px};
use std::borrow::Cow;

pub const FONT: &str = "JetBrains Mono";

pub const TITLEBAR_H: f32 = 38.0;
pub const SIDEBAR_W: f32 = 56.0;
pub const PLAYER_H: f32 = 76.0;
pub const GAP: f32 = 10.0;
pub const RADIUS: f32 = 10.0;
pub const WINDOW_RADIUS: f32 = 10.0;

/// Window background (#030303, inside the #000–#050505 band).
pub fn bg() -> Hsla {
    hsla(0.0, 0.0, 0.012, 1.0)
}
/// Floating panel fill: black at 68 %.
pub fn panel() -> Hsla {
    hsla(0.0, 0.0, 0.0, 0.68)
}
/// Denser panel (popovers, pills over busy glow): black at 75 %.
pub fn panel_strong() -> Hsla {
    hsla(0.0, 0.0, 0.0, 0.75)
}
/// Light panel: black at 60 %.
pub fn panel_light() -> Hsla {
    hsla(0.0, 0.0, 0.0, 0.60)
}
/// 1 px border, white at 12 %.
pub fn border() -> Hsla {
    hsla(0.0, 0.0, 1.0, 0.12)
}
pub fn border_strong() -> Hsla {
    hsla(0.0, 0.0, 1.0, 0.15)
}
pub fn border_faint() -> Hsla {
    hsla(0.0, 0.0, 1.0, 0.10)
}
pub fn hover() -> Hsla {
    hsla(0.0, 0.0, 1.0, 0.06)
}
pub fn active() -> Hsla {
    hsla(0.0, 0.0, 1.0, 0.10)
}
pub fn text() -> Hsla {
    hsla(0.0, 0.0, 0.93, 1.0)
}
pub fn text_dim() -> Hsla {
    hsla(0.0, 0.0, 0.93, 0.58)
}
pub fn text_faint() -> Hsla {
    hsla(0.0, 0.0, 0.93, 0.34)
}
pub fn text_ghost() -> Hsla {
    hsla(0.0, 0.0, 0.93, 0.18)
}
/// Close-button red.
pub fn danger() -> Hsla {
    hsla(355.0 / 360.0, 0.86, 0.58, 1.0)
}
pub fn ok() -> Hsla {
    hsla(140.0 / 360.0, 0.55, 0.62, 1.0)
}
pub fn warn() -> Hsla {
    hsla(40.0 / 360.0, 0.9, 0.62, 1.0)
}

pub fn text_xs() -> Pixels {
    px(10.0)
}
pub fn text_sm() -> Pixels {
    px(11.0)
}
pub fn text_md() -> Pixels {
    px(12.5)
}
pub fn text_lg() -> Pixels {
    px(16.0)
}

/// Registers the bundled JetBrains Mono (OFL, see assets/fonts/JetBrainsMono-OFL.txt).
pub fn load_fonts(cx: &App) {
    let fonts: Vec<Cow<'static, [u8]>> = vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Light.ttf").as_slice()),
    ];
    if let Err(err) = cx.text_system().add_fonts(fonts) {
        tracing::error!("failed to load bundled fonts: {err:#}");
    }
}
