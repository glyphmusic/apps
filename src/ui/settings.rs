//! Settings as a modal (Liner layout): left navigation with search and
//! grouped pages (some with sub-tabs), right the page with rows (label +
//! description left, control right). Pages: Playback, Audio, Appearance,
//! Account, Server, Integrations, Cache & Storage, Backup & share, About. The
//! search finds pages, sub-tabs and single options (see [`OPTIONS`]).

use crate::app::{BackendState, ConnTest, Glyph, SettingsPage};
use crate::i18n::{t, tf, Lang};
use crate::theme;
use crate::ui::widgets::*;
use glyph_core::config::AudioQuality;
use glyph_core::storage::{format_bytes, StorageKind};
use glyph_core::{BackendMode, UpdateStatus};
use glyph_protocol::ProviderId;
use gpui::{anchored, 
    canvas, deferred, div, hsla, point, prelude::*, px, AnyElement, Context, Div, FontWeight, Hsla,
    MouseButton, Pixels, SharedString, Window,
};
use std::rc::Rc;

mod eq;
mod controls;
mod modal;
mod nav;
mod playback;
mod sources;
mod appearance;
mod server;
mod storage;
mod about;
pub use eq::*;
pub use controls::*;
pub use modal::*;
pub use nav::*;
pub use playback::*;
pub use sources::*;
pub use appearance::*;
pub use server::*;
pub use storage::*;
pub use about::*;

pub const REPO_URL: &str = "https://github.com/glyphmusic/apps";
const DOCS_URL: &str = "https://docs.glyph.my";
const ISSUES_URL: &str = "https://github.com/glyphmusic/apps/issues/new";
const RELEASES_URL: &str = "https://github.com/glyphmusic/apps/releases";

pub(crate) const PANEL_W: f32 = 920.0;
const PANEL_H: f32 = 600.0;
pub(crate) const NAV_W: f32 = 232.0;

pub(crate) fn white(a: f32) -> Hsla {
    hsla(0.0, 0.0, 1.0, a)
}

// ------------------------------------------------------------------ pieces

pub(crate) fn switch(id: impl Into<gpui::ElementId>, on: bool) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .cursor_pointer()
        .w(px(36.0))
        .h(px(20.0))
        .rounded_full()
        .p(px(2.0))
        .flex()
        .when(on, |d| d.justify_end())
        .bg(if on { gpui::white() } else { white(0.14) })
        .child(div().size(px(16.0)).rounded_full().bg(if on { gpui::black() } else { white(0.75) }))
}

thread_local! {
    /// Option picked in the settings search: (label, since when, scrolled to).
    /// Its row lights up briefly and scrolls into view.
    static FLASH: std::cell::RefCell<Option<(String, std::time::Instant, bool)>> = const { std::cell::RefCell::new(None) };
    /// Scroll state of the settings page (for scrolling to a found option).
    static PAGE_SCROLL: std::cell::RefCell<Option<gpui::ScrollHandle>> = const { std::cell::RefCell::new(None) };
}

const FLASH_S: f32 = 1.8;

/// Highlights the row labelled `label` on the next settings page shown.
pub fn flash_option(label: impl Into<String>) {
    FLASH.with(|f| *f.borrow_mut() = Some((label.into(), std::time::Instant::now(), false)));
}

/// A found option is still lit (the app keeps rendering).
pub fn flash_animating() -> bool {
    FLASH.with(|f| f.borrow().as_ref().is_some_and(|(_, at, _)| at.elapsed().as_secs_f32() < FLASH_S))
}

/// Highlight strength (1 -> 0) for the row labelled `label`, if it is the found one.
fn flash_k(label: &str) -> Option<f32> {
    FLASH.with(|f| {
        let f = f.borrow();
        let (l, at, _) = f.as_ref()?;
        let e = at.elapsed().as_secs_f32();
        (l == label && e < FLASH_S).then(|| if e < 0.7 { 1.0 } else { 1.0 - (e - 0.7) / (FLASH_S - 0.7) })
    })
}

/// Scrolls the settings page so the found row (window bounds `b`) is in view; once.
fn scroll_to_found(b: gpui::Bounds<Pixels>) {
    let first = FLASH.with(|f| f.borrow_mut().as_mut().is_some_and(|fl| !std::mem::replace(&mut fl.2, true)));
    if !first {
        return;
    }
    PAGE_SCROLL.with(|h| {
        let Some(h) = h.borrow().clone() else { return };
        let vp = h.bounds();
        let vh = f32::from(vp.size.height);
        let top = f32::from(b.origin.y) - f32::from(vp.origin.y);
        if top >= 0.0 && top + f32::from(b.size.height) <= vh {
            return;
        }
        let max = f32::from(h.max_offset().y).max(0.0);
        let y = (f32::from(h.offset().y) - (top - vh * 0.3)).clamp(-max, 0.0);
        h.set_offset(point(px(0.0), px(y)));
    });
}

const TOUCH: bool = crate::ui::compact::TOUCH;

/// Settings row: label left with an (i) that explains it on hover, control
/// right.
pub(crate) fn row(label: impl Into<SharedString>, desc: Option<SharedString>, control: impl IntoElement) -> AnyElement {
    row_full(label, desc, None, control)
}

/// Settings row whose second line is live state (device details, connection
/// status), shown under the label instead of behind an (i).
pub(crate) fn row_status(label: impl Into<SharedString>, status: Option<SharedString>, control: impl IntoElement) -> AnyElement {
    row_full(label, None, status, control)
}

/// Settings row: label (+ (i) with `info`) and an optional `status` line left,
/// control right.
pub(crate) fn row_full(
    label: impl Into<SharedString>,
    info: Option<SharedString>,
    status: Option<SharedString>,
    control: impl IntoElement,
) -> AnyElement {
    let label: SharedString = label.into();
    let lit = flash_k(&label);
    div()
        .when_some(lit, |d, k| {
            d.relative()
                .mx(px(-10.0))
                .px(px(10.0))
                .rounded(px(8.0))
                .bg(theme::pink().opacity(0.14 * k))
                .child(canvas(|b, _, _| scroll_to_found(b), |_, _, _, _| {}).absolute().inset_0())
        })
        .flex()
        .items_center()
        .justify_between()
        .gap(px(if TOUCH { 10.0 } else { 24.0 }))
        // Phones: a wide control (field, buttons) wraps under the label
        // instead of running over it.
        .when(TOUCH, |d| d.flex_wrap())
        .py(px(14.0))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .when(TOUCH, |d| d.flex_grow(1.0).flex_basis(px(200.0)))
                .gap(px(3.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(div().min_w_0().text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(gpui::white()).child(label.clone()))
                        .when_some(info, |d, s| d.child(info_tip(SharedString::from(format!("info-{label}")), s))),
                )
                .when_some(status, |d, s| d.child(div().text_size(px(12.5)).text_color(white(0.55)).child(s))),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .when(TOUCH, |d| d.flex_grow(1.0).justify_end().flex_wrap().max_w_full())
                .when(!TOUCH, |d| d.flex_none())
                .child(control),
        )
        .into_any_element()
}

/// Rows separated by hairlines.
/// Equalizer presets: name key and gains (dB) for 32 Hz .. 16 kHz.
pub const EQ_PRESETS: [(&str, [f32; 10]); 14] = [
    ("flat", [0.0; 10]),
    ("bass_boost", [6.0, 5.0, 4.0, 2.5, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("bass_reducer", [-6.0, -5.0, -4.0, -2.5, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("treble_boost", [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.5, 4.0, 5.0, 6.0]),
    ("treble_reducer", [0.0, 0.0, 0.0, 0.0, 0.0, -1.0, -2.5, -4.0, -5.0, -6.0]),
    ("vocal", [-2.0, -2.0, -1.0, 1.0, 3.0, 4.0, 3.5, 2.0, 0.0, -1.0]),
    ("rock", [4.0, 3.0, 2.0, 0.5, -1.0, -1.0, 0.5, 2.0, 3.0, 4.0]),
    ("pop", [-1.0, 1.0, 2.5, 3.5, 2.0, 0.0, -0.5, -0.5, 1.0, 1.5]),
    ("hiphop", [5.0, 4.5, 2.0, 3.0, -1.0, -1.0, 1.0, -0.5, 1.5, 2.5]),
    ("electronic", [4.5, 4.0, 1.5, 0.0, -2.0, 2.0, 1.0, 1.5, 4.0, 5.0]),
    ("acoustic", [4.0, 4.0, 3.0, 1.0, 2.0, 1.5, 3.0, 3.5, 3.0, 2.0]),
    ("classical", [4.5, 3.5, 3.0, 2.5, -1.5, -1.5, 0.0, 2.0, 3.0, 3.5]),
    ("jazz", [3.5, 2.5, 1.5, 2.0, -1.5, -1.5, 0.0, 1.5, 2.5, 3.0]),
    ("small_speakers", [5.0, 4.0, 3.5, 2.5, 1.0, 0.0, -1.0, -2.0, -2.5, -3.0]),
];

/// Display name of a preset key: built-in (translated), own (`user:<name>`)
/// or "custom".
pub(crate) fn eq_preset_label(key: &str) -> SharedString {
    match key.strip_prefix("user:") {
        Some(name) => SharedString::from(name.to_string()),
        None => t(&format!("eq.preset.{key}")),
    }
}

/// Preset key for a curve: the current preset while it still matches, else
/// the first built-in or own preset with exactly these gains, else "custom".
pub(crate) fn eq_match(current: &str, gains: &[f32; 10], user: &[glyph_core::eq::EqPreset]) -> String {
    use glyph_core::eq::same_gains;
    let builtin = EQ_PRESETS.iter().map(|(k, g)| (k.to_string(), *g));
    let own = user.iter().map(|p| (format!("user:{}", p.name), p.gains()));
    let all: Vec<(String, [f32; 10])> = builtin.chain(own).collect();
    if all.iter().any(|(k, g)| k == current && same_gains(g, gains)) {
        return current.to_string();
    }
    all.into_iter().find(|(_, g)| same_gains(g, gains)).map(|(k, _)| k).unwrap_or_else(|| "custom".into())
}
