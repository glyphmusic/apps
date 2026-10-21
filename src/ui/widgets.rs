//! Small building blocks. Everything is flat: no gradients, no glow, no
//! shadows. ASCII only as accents (badges, spinner, empty states).

use crate::app::{track_key, Glyph};
use crate::i18n::t;
use crate::theme;
use glyph_protocol::{ProviderId, Track};
use gpui::{canvas, 
    div, img, prelude::*, px, svg, AnimationExt, AnyElement, Context, Div, ElementId, FontWeight, Hsla, ObjectFit,
    SharedString, Stateful, Svg, Window,
};
use std::rc::Rc;
use std::time::Duration;

mod tooltip;
mod buttons;
mod bars;
mod cover;
mod skeleton;
mod indicators;
mod track_row;
mod scroll;
mod fade;
pub use tooltip::*;
pub use fade::*;
pub use buttons::*;
pub use bars::*;
pub use cover::*;
pub use skeleton::*;
pub use indicators::*;
pub use track_row::*;
pub use scroll::*;

pub type Pick = Rc<dyn Fn(f32, &mut Window, &mut gpui::App)>;

/// Flat Lucide icon.
pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(SharedString::from(format!("icons/ui/{name}.svg")))
        .flex_none()
        .size(px(size))
        .text_color(color)
}

// ---- tooltips --------------------------------------------------------------
//
// Tooltips sit next to their button (below it, or above near the bottom edge
// of the window) with a small arrow pointing at it, instead of at the mouse
// where they could cover the button. Only one is open at a time. The first
// waits a moment; once one was open, the next opens at once while the mouse
// moves on within TIP_WARM_S.

const TIP_DELAY: Duration = Duration::from_millis(500);
const TIP_WARM_S: f32 = 0.8;
const TIP_BG: Hsla = Hsla { h: 0.0, s: 0.0, l: 0.12, a: 0.98 };
/// While a tooltip is open, the app re-checks this often that the mouse is
/// still on its button (a tooltip must never outlive its button).
const TIP_RECHECK: Duration = Duration::from_millis(1000);

struct TipNow {
    label: SharedString,
    /// The button, in window coordinates.
    bounds: gpui::Bounds<gpui::Pixels>,
    since: std::time::Instant,
    wait: Duration,
    /// Clicked: hidden until the mouse leaves the button.
    dismissed: bool,
    /// Long text: wraps at a fixed width instead of one line.
    wrap: bool,
}

impl TipNow {
    fn shown(&self) -> bool {
        !self.dismissed && self.since.elapsed() >= self.wait
    }
}

/// Per-window interactive UI state so multiple windows (`Glyph` main window and
/// `MiniPlayer`) never collide on tooltips, hover fades, bar drags, or scroll targets.
struct WindowUiState {
    tip_now: Option<TipNow>,
    tip_last: Option<std::time::Instant>,
    /// The open tooltip's button was painted (and hovered) in the last frame.
    tip_painted: bool,
    mouse_out: bool,
    hovers: std::collections::HashMap<SharedString, (bool, std::time::Instant, f32)>,
    bar_drag: Option<(&'static str, f32)>,
    bar_hover: Option<&'static str>,
    bar_hover_at: f32,
    viewport_h: f32,
    smooth: Option<(gpui::ScrollHandle, f32, f32)>,
    smooth_at: Option<std::time::Instant>,
    wheel_frames: Vec<(gpui::Bounds<gpui::Pixels>, bool)>,
    last_wheel: Option<(gpui::Bounds<gpui::Pixels>, std::time::Instant)>,
    scroll_drag: Option<(gpui::ScrollHandle, f32, f32)>,
    scroll_seen: std::collections::HashMap<(i32, i32), (f32, Option<std::time::Instant>, bool)>,
    vlist_top: std::collections::HashMap<&'static str, f32>,
}

impl Default for WindowUiState {
    fn default() -> Self {
        Self {
            tip_now: None,
            tip_last: None,
            tip_painted: false,
            mouse_out: false,
            hovers: Default::default(),
            bar_drag: None,
            bar_hover: None,
            bar_hover_at: 0.0,
            viewport_h: 900.0,
            smooth: None,
            smooth_at: None,
            wheel_frames: Vec::new(),
            last_wheel: None,
            scroll_drag: None,
            scroll_seen: Default::default(),
            vlist_top: Default::default(),
        }
    }
}

struct UiRuntime {
    windows: std::collections::HashMap<gpui::WindowId, WindowUiState>,
    current_win: Option<gpui::WindowId>,
    fallback_win: WindowUiState,
    ascii_fx: bool,
    skeleton_seen: bool,
    /// A "now playing" equalizer was drawn (the app redraws it ~15 times a second).
    eq_seen: bool,
    t0: std::time::Instant,
    shimmer_glyphs: std::collections::HashMap<(u8, u32), gpui::ShapedLine>,
    last_toggle: Option<(SharedString, std::time::Instant)>,
    pressed: std::collections::HashMap<&'static str, std::time::Instant>,
    reveals: std::collections::HashMap<String, std::time::Instant>,
    handles: std::collections::HashMap<String, gpui::ScrollHandle>,
    drag_shift: std::collections::HashMap<
        &'static str,
        (std::collections::HashMap<usize, (f32, std::time::Instant)>, Option<std::time::Instant>),
    >,
}

impl Default for UiRuntime {
    fn default() -> Self {
        Self {
            windows: Default::default(),
            current_win: None,
            fallback_win: WindowUiState::default(),
            ascii_fx: true,
            skeleton_seen: false,
            eq_seen: false,
            t0: std::time::Instant::now(),
            shimmer_glyphs: Default::default(),
            last_toggle: None,
            pressed: Default::default(),
            reveals: Default::default(),
            handles: Default::default(),
            drag_shift: Default::default(),
        }
    }
}

impl UiRuntime {
    fn win(&self, id: gpui::WindowId) -> Option<&WindowUiState> {
        self.windows.get(&id)
    }

    fn win_mut(&mut self, id: gpui::WindowId) -> &mut WindowUiState {
        self.windows.entry(id).or_default()
    }

    fn cur_win(&self) -> &WindowUiState {
        self.current_win.and_then(|id| self.windows.get(&id)).unwrap_or(&self.fallback_win)
    }

    fn cur_win_mut(&mut self) -> &mut WindowUiState {
        if let Some(id) = self.current_win {
            self.windows.entry(id).or_default()
        } else {
            &mut self.fallback_win
        }
    }
}

thread_local! {
    static UI_RT: std::cell::RefCell<UiRuntime> = std::cell::RefCell::new(UiRuntime::default());
}

/// Click-catching backdrop of a modal. It starts below the titlebar, so the
/// window can still be dragged (and the window buttons used) while it is open;
/// put it in `modal_layer`, which dims the titlebar without catching clicks.
pub fn modal_backdrop(id: impl Into<ElementId>, dim: f32) -> Stateful<Div> {
    div()
        .id(id.into())
        .occlude()
        .absolute()
        .top(px(theme::TITLEBAR_H))
        .left_0()
        .right_0()
        .bottom_0()
        // Keeps the panel centered on the window, not below the titlebar.
        .pb(px(theme::TITLEBAR_H))
        .bg(gpui::hsla(0.0, 0.0, 0.0, dim))
        .flex()
        .items_center()
        .justify_center()
}

/// Full-window modal layer: a dimmed titlebar that lets clicks through, and
/// the `modal_backdrop` below it.
pub fn modal_layer(dim: f32, backdrop: impl IntoElement) -> Div {
    div()
        .absolute()
        .inset_0()
        .child(div().absolute().top_0().left_0().right_0().h(px(theme::TITLEBAR_H)).bg(gpui::hsla(0.0, 0.0, 0.0, dim)))
        .child(backdrop)
}

/// Selects `window` as the active window for the current render pass and resets
/// its per-frame wheel target list and viewport height.
pub fn begin_window_frame(window: &Window) {
    let id = window.window_handle().window_id();
    let vh = f32::from(window.viewport_size().height);
    UI_RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        rt.current_win = Some(id);
        let w = rt.win_mut(id);
        w.viewport_h = vh;
        w.wheel_frames.clear();
        // The open tooltip's button was not drawn (or not hovered) last
        // frame: it is gone (page changed, dialog on top), so is the tooltip.
        if w.tip_now.is_some() && !w.tip_painted {
            if w.tip_now.as_ref().is_some_and(TipNow::shown) {
                w.tip_last = Some(std::time::Instant::now());
            }
            w.tip_now = None;
        }
        w.tip_painted = false;
    });
}

/// A tooltip is open or one closed a moment ago: the next opens at once.
pub fn tips_warm() -> bool {
    UI_RT.with(|rt| {
        let w = rt.borrow();
        let w = w.cur_win();
        w.tip_now.as_ref().is_some_and(TipNow::shown)
            || w.tip_last.is_some_and(|at| at.elapsed().as_secs_f32() < TIP_WARM_S)
    })
}

/// A tooltip just closed (also the sidebar's own ones).
pub fn tip_closed() {
    UI_RT.with(|rt| rt.borrow_mut().cur_win_mut().tip_last = Some(std::time::Instant::now()));
}

const PRESS_SECS: f32 = 0.22;

/// Marks a button as just pressed.
pub fn press(id: &'static str) {
    UI_RT.with(|rt| {
        rt.borrow_mut().pressed.insert(id, std::time::Instant::now());
    });
}

/// 1 right after a press, easing back to 0.
pub fn press_k(id: &'static str) -> f32 {
    UI_RT.with(|rt| {
        rt.borrow()
            .pressed
            .get(id)
            .map_or(0.0, |t| (1.0 - t.elapsed().as_secs_f32() / PRESS_SECS).clamp(0.0, 1.0).powi(2))
    })
}

/// A press animation is still running (keeps frames coming).
pub fn press_running() -> bool {
    UI_RT.with(|rt| rt.borrow().pressed.values().any(|t| t.elapsed().as_secs_f32() < PRESS_SECS))
}
