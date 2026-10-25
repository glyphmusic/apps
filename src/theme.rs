//! Glyph's visual tokens: deep black, floating translucent panels, hairline
//! borders, no shadows, monospace everywhere.

use gpui::{canvas, div, hsla, prelude::*, px, AnyElement, App, Bounds, Hsla, Pixels};
use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Instant;
use crate::visuals::Rgb;

/// Body / titles.
pub const FONT: &str = "Inter";
/// Accents only: times, badges, status lines, logo.
pub const MONO: &str = "JetBrains Mono";

pub const TITLEBAR_H: f32 = 44.0;
pub const SIDEBAR_W: f32 = 64.0;
pub const PLAYER_H: f32 = 80.0;
pub const GAP: f32 = 10.0;
pub const RADIUS: f32 = 10.0;
/// Horizontal padding of the main pages: roomy on the desktop, tight on
/// phones so content uses the full width (plus the layout's GAP).
pub const PAGE_X: f32 = if cfg!(any(target_os = "android", target_os = "ios")) { 16.0 } else { 32.0 };
/// Space above a page's first item. Phones already have a header row above
/// the page (its own padding below), so they add only a little.
pub const PAGE_TOP: f32 = if cfg!(any(target_os = "android", target_os = "ios")) { 8.0 } else { 20.0 };
pub const WINDOW_RADIUS: f32 = 10.0;

// ---- colour themes -------------------------------------------------------
//
// A theme sets the accent (everything that was Glyph pink), the glow colours
// while no song is playing (a song's own colours still take over), and a
// faint tint in the black base. Picked in Settings -> Appearance.

pub struct Theme {
    pub id: &'static str,
    /// i18n key of its name.
    pub name: &'static str,
    pub accent: u32,
    /// Glow colours dark -> bright; `None`: each view's own colours.
    pub glow: Option<[u32; 3]>,
    pub base: u32,
}

pub const THEMES: [Theme; 7] = [
    Theme { id: "glyph", name: "theme.glyph", accent: 0xff5fd2, glow: None, base: 0x030303 },
    Theme { id: "ember", name: "theme.ember", accent: 0xff8a3d, glow: Some([0x3a1206, 0xff6516, 0xffc04a]), base: 0x050302 },
    Theme { id: "ocean", name: "theme.ocean", accent: 0x4da3ff, glow: Some([0x06124a, 0x1d5cff, 0x46dcff]), base: 0x020308 },
    Theme { id: "forest", name: "theme.forest", accent: 0x3ddc84, glow: Some([0x04210f, 0x1f9d55, 0x9cf0b8]), base: 0x020503 },
    Theme { id: "violet", name: "theme.violet", accent: 0xb07cff, glow: Some([0x16052f, 0x6526d6, 0xb88dff]), base: 0x040208 },
    Theme { id: "crimson", name: "theme.crimson", accent: 0xff4d5e, glow: Some([0x360404, 0xb11d0b, 0xff6a4d]), base: 0x060202 },
    Theme { id: "mono", name: "theme.mono", accent: 0xe8e8e8, glow: Some([0x101010, 0x5a5a5a, 0xd0d0d0]), base: 0x030303 },
];

/// The theme with `id` (unknown ids: the default).
pub fn theme_by_id(id: &str) -> &'static Theme {
    THEMES.iter().find(|t| t.id == id).unwrap_or(&THEMES[0])
}

/// A theme from an installed plugin (Settings -> Plugins); its config id is
/// `plugin:<plugin id>`.
#[derive(Clone)]
pub struct PluginTheme {
    pub id: String,
    pub name: String,
    pub colors: glyph_core::plugins::ThemeColors,
}

static PLUGIN_THEMES: std::sync::RwLock<Vec<PluginTheme>> = std::sync::RwLock::new(Vec::new());

/// Replaces the plugin themes (after start-up, install, update, removal).
pub fn set_plugin_themes(themes: Vec<PluginTheme>) {
    *PLUGIN_THEMES.write().unwrap_or_else(|e| e.into_inner()) = themes;
}

pub fn plugin_themes() -> Vec<PluginTheme> {
    PLUGIN_THEMES.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Colours of theme `id`, built-in or from a plugin (unknown: the default,
/// e.g. a plugin theme synced from a device that has the plugin).
pub fn colors(id: &str) -> glyph_core::plugins::ThemeColors {
    if id.starts_with("plugin:") {
        if let Some(t) = PLUGIN_THEMES.read().unwrap_or_else(|e| e.into_inner()).iter().find(|t| t.id == id) {
            return t.colors;
        }
    }
    let t = theme_by_id(id);
    glyph_core::plugins::ThemeColors { accent: t.accent, glow: t.glow, base: t.base }
}

/// `id` names a built-in or an installed plugin theme.
pub fn known_theme(id: &str) -> bool {
    THEMES.iter().any(|t| t.id == id) || PLUGIN_THEMES.read().unwrap_or_else(|e| e.into_inner()).iter().any(|t| t.id == id)
}

#[derive(Default)]
struct Scope {
    bounds: Option<Bounds<Pixels>>,
    ink: f32,
}

struct ThemeRuntime {
    accent: Hsla,
    base: Hsla,
    zone: Option<usize>,
    ink: [f32; 3],
    scope: Option<u64>,
    scopes: HashMap<u64, Scope>,
    grid: Option<Arc<LumGrid>>,
    last_ease: Option<Instant>,
}

impl Default for ThemeRuntime {
    fn default() -> Self {
        Self {
            accent: gpui::rgb(0xff5fd2).into(),
            base: gpui::rgb(0x030303).into(),
            zone: None,
            ink: [0.0; 3],
            scope: None,
            scopes: HashMap::new(),
            grid: None,
            last_ease: None,
        }
    }
}

thread_local! {
    static THEME_RT: std::cell::RefCell<ThemeRuntime> = std::cell::RefCell::new(ThemeRuntime::default());
}

/// The theme's glow colours as a palette for the background.
pub fn glow_palette(id: &str) -> Option<crate::visuals::Palette> {
    colors(id).glow.map(|[a, b, c]| crate::visuals::Palette([Rgb::hex(a), Rgb::hex(b), Rgb::hex(c)]))
}

/// Makes `id` the current theme (set before each frame).
pub fn set_theme(id: &str) {
    let t = colors(id);
    THEME_RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        rt.accent = gpui::rgb(t.accent).into();
        rt.base = gpui::rgb(t.base).into();
    });
}

/// Window background (the theme's near-black base).
pub fn bg() -> Hsla {
    THEME_RT.with(|rt| rt.borrow().base)
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
// ---- adaptive text ink -------------------------------------------------
//
// Text that sits directly on the glow adapts to it: the glow worker reports
// the displayed luminance per zone, `Glyph` eases an "ink" value per zone
// (0 = light text, 1 = dark text) and sets it here before building each zone.
// Text on flat panels/cards uses zone `None` and always stays light.

pub fn set_ink(ink: [f32; 3]) {
    THEME_RT.with(|rt| rt.borrow_mut().ink = ink);
}

/// `Some(zone index)` for text on the background, `None` on panels.
pub fn set_zone(zone: Option<usize>) {
    THEME_RT.with(|rt| rt.borrow_mut().zone = zone);
}

/// Runs `f` with panel ink (light text), restoring the previous zone/scope.
pub fn on_panel<R>(f: impl FnOnce() -> R) -> R {
    let (prev_zone, prev_scope) = THEME_RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        (rt.zone.take(), rt.scope.take())
    });
    let r = f();
    THEME_RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        rt.zone = prev_zone;
        rt.scope = prev_scope;
    });
    r
}

fn ink() -> f32 {
    THEME_RT.with(|rt| {
        let rt = rt.borrow();
        if let Some(k) = rt.scope {
            return rt.scopes.get(&k).map(|e| e.ink).unwrap_or(0.0);
        }
        rt.zone.map(|i| rt.ink[i.min(2)]).unwrap_or(0.0)
    })
}

// ---- per-element adaptive ink --------------------------------------------
//
// Zones are too coarse for text in the content column (a heading next to the
// bright beam got dark ink on a dark spot). `adaptive` measures the
// background under each wrapped element instead: its bounds are recorded at
// prepaint, and `update_inks` eases that element's ink towards the colour with
// the better contrast against the glow luminance under exactly those bounds.

/// Displayed background luminance on a coarse grid, in window coordinates.
pub struct LumGrid {
    pub w: usize,
    pub h: usize,
    /// Logical px per cell.
    pub pitch: f32,
    pub data: Vec<f32>,
}

impl LumGrid {
    /// Bright end (90th percentile) of the luminance under `b`.
    fn sample(&self, b: Bounds<Pixels>) -> Option<f32> {
        let x0 = (f32::from(b.origin.x) / self.pitch).floor().max(0.0) as usize;
        let y0 = (f32::from(b.origin.y) / self.pitch).floor().max(0.0) as usize;
        let x1 = ((f32::from(b.origin.x + b.size.width) / self.pitch).ceil() as usize).min(self.w);
        let y1 = ((f32::from(b.origin.y + b.size.height) / self.pitch).ceil() as usize).min(self.h);
        let mut v: Vec<f32> = Vec::new();
        for y in y0..y1 {
            v.extend_from_slice(&self.data[y * self.w + x0.min(x1)..y * self.w + x1]);
        }
        if v.is_empty() {
            return None;
        }
        let k = ((v.len() as f32 * 0.9) as usize).min(v.len() - 1);
        v.select_nth_unstable_by(k, |a, b| a.total_cmp(b));
        Some(v[k])
    }
}

/// Wraps `build()` so every theme text colour inside it adapts to the
/// background under the element. `key` must be stable across frames.
pub fn adaptive(key: impl Hash, build: impl FnOnce() -> AnyElement) -> AnyElement {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut h);
    let k = h.finish();
    let prev = THEME_RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        if rt.scopes.len() > 1024 && !rt.scopes.contains_key(&k) {
            rt.scopes.retain(|_, s| s.bounds.is_some());
            if rt.scopes.len() > 1024 {
                rt.scopes.clear();
            }
        }
        rt.scopes.entry(k).or_default();
        rt.scope.replace(k)
    });
    let el = build();
    THEME_RT.with(|rt| rt.borrow_mut().scope = prev);
    div()
        .relative()
        .child(
            canvas(
                move |bounds, _, _| {
                    THEME_RT.with(|rt| {
                        if let Some(e) = rt.borrow_mut().scopes.get_mut(&k) {
                            e.bounds = Some(bounds);
                        }
                    })
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
        .child(el)
        .into_any_element()
}

pub fn set_lum_grid(grid: Option<Arc<LumGrid>>) {
    THEME_RT.with(|rt| rt.borrow_mut().grid = grid);
}

/// Eases every adaptive element's ink (~300 ms). Returns true while anything
/// still changes, so the caller re-renders.
pub fn update_inks() -> bool {
    let now = Instant::now();
    THEME_RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        let dt = rt
            .last_ease
            .replace(now)
            .map(|t| (now - t).as_secs_f32().min(0.1))
            .unwrap_or(0.016);
        let step = dt / 0.3;
        let Some(grid) = rt.grid.clone() else { return false };
        let mut changed = false;
        for e in rt.scopes.values_mut() {
            let Some(lum) = e.bounds.and_then(|b| grid.sample(b)) else { continue };
            let target = ink_for(lum);
            if (e.ink - target).abs() > 0.001 {
                e.ink = if target > e.ink { (e.ink + step).min(target) } else { (e.ink - step).max(target) };
                changed = true;
            }
        }
        changed
    })
}

/// Ink for a background brightness: 0 light text, 1 dark text.
///
/// `lum` is the gamma-encoded (sRGB) luma the glow worker reports; WCAG
/// contrast needs linear luminance, so convert first. White text stays until
/// the background is genuinely bright (linear 0.30, white still ≈ 3:1 there),
/// because dark text on a mid-grey glow reads far worse than white.
pub fn ink_for(lum: f32) -> f32 {
    let v = lum.clamp(0.0, 1.0);
    let linear = if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    if linear > 0.30 { 1.0 } else { 0.0 }
}

fn ink_color(alpha_light: f32, alpha_dark: f32) -> Hsla {
    let k = ink();
    hsla(0.0, 0.0, 0.95 + (0.05 - 0.95) * k, alpha_light + (alpha_dark - alpha_light) * k)
}

pub fn text() -> Hsla {
    ink_color(1.0, 1.0)
}
pub fn text_dim() -> Hsla {
    ink_color(0.72, 0.82)
}
pub fn text_faint() -> Hsla {
    ink_color(0.56, 0.70)
}
pub fn text_ghost() -> Hsla {
    ink_color(0.22, 0.3)
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

/// Text size factor (Settings > Appearance), as f32 bits.
static TEXT_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

/// Sets the text size factor (1.0 = normal).
pub fn set_text_scale(k: f32) {
    let k = if k.is_finite() { k.clamp(0.9, 1.3) } else { 1.0 };
    TEXT_SCALE.store(k.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// The text size factor (1.0 = normal).
pub fn text_scale() -> f32 {
    f32::from_bits(TEXT_SCALE.load(std::sync::atomic::Ordering::Relaxed))
}

pub fn text_xs() -> Pixels {
    px(11.5 * text_scale())
}
pub fn text_sm() -> Pixels {
    px(12.5 * text_scale())
}
pub fn text_md() -> Pixels {
    px(13.5 * text_scale())
}
pub fn text_lg() -> Pixels {
    px(16.0 * text_scale())
}

/// The accent colour of the theme (Glyph pink by default).
pub fn pink() -> Hsla {
    THEME_RT.with(|rt| rt.borrow().accent)
}

/// Dark tile in the accent's hue (Liked Songs, Downloads covers).
pub fn accent_tile(l: f32, s: f32) -> Hsla {
    let a = pink();
    hsla(a.h, if a.s < 0.05 { 0.0 } else { s }, l, 1.0)
}

/// Flat card fill (#141414-like at 75 %).
pub fn card() -> Hsla {
    hsla(0.0, 0.0, 0.08, 0.75)
}
pub fn card_hover() -> Hsla {
    hsla(0.0, 0.0, 0.12, 0.82)
}

/// Registers the bundled Inter and JetBrains Mono (both OFL, see assets/fonts/*-OFL.txt).
pub fn load_fonts(cx: &App) {
    let fonts: Vec<Cow<'static, [u8]>> = vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Light.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Regular.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Medium.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-SemiBold.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Bold.ttf").as_slice()),
    ];
    if let Err(err) = cx.text_system().add_fonts(fonts) {
        tracing::error!("failed to load bundled fonts: {err:#}");
    }
}
