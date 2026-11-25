//! Dynamic theme accent resolution and app icon recoloring.
//!
//! Recolors the pink accent pixels of Glyph's bundled icon bitmaps so the
//! desktop window, tray, macOS Dock, and Android Recents match the selected
//! built-in or plugin theme.

use crate::controller::Core;
use std::io::Cursor;

/// Default Glyph pink accent (`#ff5fd2`).
pub const DEFAULT_ACCENT: u32 = 0xff5fd2;

/// Built-in theme IDs mapped to their `#rrggbb` accent colour (kept in sync
/// with `glyph-ui/src/lib/theme.ts`).
pub const BUILTIN_ACCENTS: &[(&str, u32)] = &[
    ("glyph", 0xff5fd2),
    ("ember", 0xff8a3d),
    ("ocean", 0x4da3ff),
    ("forest", 0x3ddc84),
    ("violet", 0xb07cff),
    ("crimson", 0xff4d5e),
    ("mono", 0xe8e8e8),
];

/// Returns the accent colour for a built-in theme ID.
pub fn builtin_accent(theme_id: &str) -> Option<u32> {
    BUILTIN_ACCENTS.iter().find(|(id, _)| *id == theme_id).map(|(_, accent)| *accent)
}

/// Resolves the active accent colour (`0xrrggbb`) for a built-in or
/// `plugin:<id>` theme.
pub fn resolve_accent(core: &Core, theme_id: &str) -> u32 {
    if let Some(pid) = theme_id.strip_prefix("plugin:") {
        if let Some(st) = core.plugins() {
            if let Some(accent) = crate::plugins::theme_accent(&st, pid) {
                return accent;
            }
        }
    }
    builtin_accent(theme_id).unwrap_or(DEFAULT_ACCENT)
}

/// Notifies the host platform of the currently configured theme accent and
/// a plugin look's app icon (if one is in use) so native icons (window,
/// tray, Dock, Android launcher/Recents) stay in sync.
pub fn sync_host_icon(core: &Core) {
    let (theme_id, disabled) = {
        let s = core.st();
        (s.config.visuals.theme.clone(), s.config.plugins.disabled.clone())
    };
    let accent = resolve_accent(core, &theme_id);
    let icon = core.plugins().and_then(|st| crate::plugins::active_app_icon(&st, &disabled, &theme_id));
    tracing::debug!(theme = %theme_id, plugins = core.plugins().is_some(), plugin_icon = icon.as_ref().map(Vec::len), "app icon");
    core.host.set_theme_icon(&theme_id, accent, icon.as_deref());
}

/// Decodes PNG bytes into `(width, height, rgba8)` pixels.
pub fn decode_png(png: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some((w, h, rgba.into_raw()))
}

/// Decodes a plugin's app icon for window, tray and Dock: at most `max`
/// pixels a side. Big icons (512, 1024) are scaled down: some systems drop
/// a window icon that large (X11 kept no icon at all for 512 x 512).
pub fn decode_icon(png: &[u8], max: u32) -> Option<(u32, u32, Vec<u8>)> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?;
    let img = if img.width() > max || img.height() > max { img.resize(max, max, image::imageops::FilterType::Lanczos3) } else { img };
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some((w, h, rgba.into_raw()))
}

/// Encodes `(width, height, rgba8)` pixels into PNG bytes.
pub fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let img = image::RgbaImage::from_raw(w, h, rgba.to_vec())?;
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img).write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

/// Recolors the pink accent pixels in an RGBA8 icon buffer to `accent`
/// (`0xrrggbb`), preserving black background, white glyphs, and anti-aliasing.
pub fn tint_rgba(rgba: &[u8], accent: u32) -> Vec<u8> {
    if accent == DEFAULT_ACCENT {
        return rgba.to_vec();
    }
    let tr = ((accent >> 16) & 0xff) as f32;
    let tg = ((accent >> 8) & 0xff) as f32;
    let tb = (accent & 0xff) as f32;
    // Matches the pastel highlight balance of the original pink icon glyphs.
    const MIX: f32 = 0.72;
    let sr = (MIX * tr + (1.0 - MIX) * 255.0) / 255.0;
    let sg = (MIX * tg + (1.0 - MIX) * 255.0) / 255.0;
    let sb = (MIX * tb + (1.0 - MIX) * 255.0) / 255.0;

    let mut out = rgba.to_vec();
    for px in out.chunks_exact_mut(4) {
        let (r, g, b, a) = (px[0] as f32, px[1] as f32, px[2] as f32, px[3]);
        if a == 0 || r <= g || b <= g {
            continue;
        }
        let intensity = r.min(((r - g) / 0.3125).min((b - g) / 0.256));
        if intensity <= 0.5 {
            continue;
        }
        let white = (r - intensity).max(0.0);
        px[0] = (white + intensity * sr).clamp(0.0, 255.0).round() as u8;
        px[1] = (white + intensity * sg).clamp(0.0, 255.0).round() as u8;
        px[2] = (white + intensity * sb).clamp(0.0, 255.0).round() as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_accent_is_unchanged() {
        let src = vec![0, 0, 0, 255, 255, 255, 255, 255, 255, 175, 232, 255];
        assert_eq!(tint_rgba(&src, DEFAULT_ACCENT), src);
    }

    #[test]
    fn neutral_and_transparent_pixels_stay_intact() {
        let src = vec![
            0, 0, 0, 255,       // pure black background
            255, 255, 255, 255, // pure white ASCII character
            255, 175, 232, 0,   // transparent pixel
        ];
        let tinted = tint_rgba(&src, 0x4da3ff);
        assert_eq!(tinted, src);
    }

    #[test]
    fn pink_pixel_tints_toward_target_hue() {
        let src = vec![255, 175, 232, 255];
        let ocean = tint_rgba(&src, 0x4da3ff);
        assert!(ocean[2] > ocean[1] && ocean[1] > ocean[0], "expected blue-dominant pixel, got {ocean:?}");

        let ember = tint_rgba(&src, 0xff8a3d);
        assert!(ember[0] > ember[1] && ember[1] > ember[2], "expected orange-dominant pixel, got {ember:?}");

        let mono = tint_rgba(&src, 0xe8e8e8);
        assert_eq!(mono[0], mono[1]);
        assert_eq!(mono[1], mono[2]);
    }

    #[test]
    fn big_plugin_icons_are_scaled_down() {
        let big = encode_png(512, 512, &vec![200; 512 * 512 * 4]).unwrap();
        let (w, h, px) = decode_icon(&big, 256).unwrap();
        assert_eq!((w, h, px.len()), (256, 256, 256 * 256 * 4));
        let small = encode_png(128, 128, &vec![9; 128 * 128 * 4]).unwrap();
        assert_eq!(decode_icon(&small, 256).unwrap().0, 128);
    }

    #[test]
    fn builtin_accents_resolve() {
        assert_eq!(builtin_accent("glyph"), Some(DEFAULT_ACCENT));
        assert_eq!(builtin_accent("ocean"), Some(0x4da3ff));
        assert_eq!(builtin_accent("unknown"), None);
    }
}
