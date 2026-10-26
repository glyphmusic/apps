//! The activity's surface as GPUI's one full-screen window.
//!
//! The native window comes and goes with the activity (`InitWindow` /
//! `TerminateWindow`); the wgpu renderer survives that: its surface is
//! unconfigured while the app is in the background and replaced when it
//! returns, so atlas textures and pipelines stay valid.
//!
//! Frames are drawn on demand: GPUI's frame waker marks the window as
//! wanting one, the run loop draws it at the next frame slot and otherwise
//! sleeps. Slots follow the display rate while a finger is on the screen
//! (and shortly after, for flings) and are capped at 60 Hz otherwise, so
//! timer-driven motion does not keep a 120 Hz panel's GPU busy.

use crate::display::AndroidDisplay;
use crate::input::{self, ImeSync};
use crate::{CallbackSlot, java};
use android_activity::AndroidApp;
use android_activity::input::{ImeOptions, InputType, TextInputAction as AndroidTextAction};
use gpui::{
    AnyWindowHandle, Bounds, Capslock, DevicePixels, DispatchEventResult, Edges, GpuSpecs, Modifiers,
    Pixels, PlatformAtlas, PlatformDisplay, PlatformInput, PlatformInputHandler, PlatformWindow, Point,
    PromptButton, PromptLevel, RequestFrameOptions, Scene, Size, TextInputAction, TextInputConfiguration,
    TextInputStateChange, WindowAppearance, WindowBackgroundAppearance, WindowBounds, WindowControlArea,
    WindowInsets, WindowParams, WindowVisibility, px, size,
};
use gpui_wgpu::{GpuContext, WgpuRenderer, WgpuSurfaceConfig};
use ndk::native_window::NativeWindow;
use raw_window_handle::{
    AndroidDisplayHandle, DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Frame-rate cap while nobody touches the screen.
const CALM_MAX_HZ: f32 = 60.0;
/// Full display rate for this long after the last touch (flings, settling).
const TOUCH_FULL_RATE: Duration = Duration::from_millis(1500);
/// A slot this close counts as reached (the looper may wake a bit early).
const SLOT_SLACK: Duration = Duration::from_millis(1);

/// `NativeWindow` with a display handle, as wgpu wants both.
#[derive(Debug, Clone)]
pub(crate) struct RawWindow(pub NativeWindow);

impl HasWindowHandle for RawWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.0.window_handle()
    }
}

impl HasDisplayHandle for RawWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::android())
    }
}

fn surface_config(native: &NativeWindow) -> WgpuSurfaceConfig {
    WgpuSurfaceConfig {
        size: size(DevicePixels(native.width().max(1)), DevicePixels(native.height().max(1))),
        transparent: false,
        // FIFO can block in get_current_texture() while the activity is
        // being backgrounded; mailbox does not.
        preferred_present_mode: Some(gpui_wgpu::wgpu::PresentMode::Mailbox),
    }
}

pub(crate) struct AndroidWindow(pub(crate) Rc<AndroidWindowState>);

pub(crate) struct AndroidWindowState {
    app: AndroidApp,
    gpu_context: GpuContext,
    renderer: RefCell<WgpuRenderer>,
    native: RefCell<Option<RawWindow>>,
    /// Logical size and scale (density).
    size: Cell<Size<Pixels>>,
    scale: Cell<f32>,
    refresh_hz: Cell<f32>,
    insets: RefCell<WindowInsets>,
    appearance: Cell<WindowAppearance>,
    active: Cell<bool>,
    visibility: Cell<WindowVisibility>,
    force_next_frame: Cell<bool>,
    /// GPUI wants a frame (set by its frame waker and `schedule_frame`).
    frame_wanted: Rc<Cell<bool>>,
    last_frame: Cell<Option<Instant>>,
    last_touch: Cell<Option<Instant>>,
    mouse_position: Cell<Point<Pixels>>,
    modifiers: Cell<Modifiers>,
    pub(crate) touches: RefCell<input::TouchIds>,
    /// Touches copied this run-loop pass, delivered merged (see `input`).
    pub(crate) motions: RefCell<input::PendingMotions>,
    pub(crate) ime: RefCell<ImeSync>,
    back_enabled: Cell<bool>,
    /// The last back press went to GPUI (so its release does too).
    pub(crate) back_taken: Cell<bool>,
    back_handler: CallbackSlot<Box<dyn FnMut()>>,
    input_handler: CallbackSlot<PlatformInputHandler>,
    request_frame_callback: CallbackSlot<Box<dyn FnMut(RequestFrameOptions)>>,
    input_callback: CallbackSlot<Box<dyn FnMut(PlatformInput) -> DispatchEventResult>>,
    active_status_callback: CallbackSlot<Box<dyn FnMut(bool)>>,
    visibility_callback: CallbackSlot<Box<dyn FnMut(WindowVisibility)>>,
    hover_status_callback: CallbackSlot<Box<dyn FnMut(bool)>>,
    resize_callback: CallbackSlot<Box<dyn FnMut(Size<Pixels>, f32)>>,
    moved_callback: CallbackSlot<Box<dyn FnMut()>>,
    display_changed_callback: CallbackSlot<Box<dyn FnMut()>>,
    should_close_callback: CallbackSlot<Box<dyn FnMut() -> bool>>,
    hit_test_callback: CallbackSlot<Box<dyn FnMut() -> Option<WindowControlArea>>>,
    close_callback: CallbackSlot<Box<dyn FnOnce()>>,
    appearance_changed_callback: CallbackSlot<Box<dyn FnMut()>>,
    insets_changed_callback: CallbackSlot<Box<dyn FnMut(WindowInsets)>>,
}

impl AndroidWindow {
    pub fn new(
        _handle: AnyWindowHandle,
        _params: WindowParams,
        app: &AndroidApp,
        gpu_context: GpuContext,
    ) -> anyhow::Result<Self> {
        let native = app
            .native_window()
            .ok_or_else(|| anyhow::anyhow!("the activity has no native window yet"))?;
        let raw = RawWindow(native.clone());
        let renderer = WgpuRenderer::new(gpu_context.clone(), &raw, surface_config(&native), None)?;
        let scale = density_scale(app);
        let state = AndroidWindowState {
            app: app.clone(),
            gpu_context,
            renderer: RefCell::new(renderer),
            native: RefCell::new(Some(raw)),
            size: Cell::new(logical_size(&native, scale)),
            scale: Cell::new(scale),
            refresh_hz: Cell::new(java::refresh_rate().unwrap_or(60.0)),
            insets: RefCell::new(crate::current_insets(scale)),
            appearance: Cell::new(appearance(app)),
            active: Cell::new(true),
            visibility: Cell::new(WindowVisibility::Visible),
            force_next_frame: Cell::new(true),
            frame_wanted: Rc::new(Cell::new(true)),
            last_frame: Cell::new(None),
            last_touch: Cell::new(None),
            mouse_position: Cell::new(Point::default()),
            modifiers: Cell::new(Modifiers::default()),
            touches: RefCell::default(),
            motions: RefCell::default(),
            ime: RefCell::default(),
            back_enabled: Cell::new(false),
            back_taken: Cell::new(false),
            back_handler: CallbackSlot::default(),
            input_handler: CallbackSlot::default(),
            request_frame_callback: CallbackSlot::default(),
            input_callback: CallbackSlot::default(),
            active_status_callback: CallbackSlot::default(),
            visibility_callback: CallbackSlot::default(),
            hover_status_callback: CallbackSlot::default(),
            resize_callback: CallbackSlot::default(),
            moved_callback: CallbackSlot::default(),
            display_changed_callback: CallbackSlot::default(),
            should_close_callback: CallbackSlot::default(),
            hit_test_callback: CallbackSlot::default(),
            close_callback: CallbackSlot::default(),
            appearance_changed_callback: CallbackSlot::default(),
            insets_changed_callback: CallbackSlot::default(),
        };
        Ok(Self(Rc::new(state)))
    }
}

fn density_scale(app: &AndroidApp) -> f32 {
    app.config().density().map_or(1.0, |dpi| dpi as f32 / 160.0).max(0.5)
}

fn logical_size(native: &NativeWindow, scale: f32) -> Size<Pixels> {
    size(px(native.width() as f32 / scale), px(native.height() as f32 / scale))
}

fn appearance(app: &AndroidApp) -> WindowAppearance {
    match app.config().ui_mode_night() {
        ndk::configuration::UiModeNight::Yes => WindowAppearance::Dark,
        _ => WindowAppearance::Light,
    }
}

impl AndroidWindowState {
    pub(crate) fn display(&self) -> AndroidDisplay {
        AndroidDisplay {
            bounds: Bounds::new(Point::default(), self.size.get()),
            refresh_hz: self.refresh_hz.get(),
        }
    }

    pub(crate) fn scale(&self) -> f32 {
        self.scale.get()
    }

    pub(crate) fn has_surface(&self) -> bool {
        self.native.borrow().is_some() && self.visibility.get() == WindowVisibility::Visible
    }

    /// How long the run loop may sleep before the next frame; `None` while
    /// no frame is wanted (it then sleeps until an event or a wake).
    pub(crate) fn next_frame_in(&self) -> Option<Duration> {
        let wanted = self.force_next_frame.get() || self.frame_wanted.get();
        (wanted && self.active.get() && self.has_surface()).then(|| self.frame_wait())
    }

    fn frame_wait(&self) -> Duration {
        let Some(last) = self.last_frame.get() else { return Duration::ZERO };
        self.frame_interval().saturating_sub(last.elapsed()).saturating_sub(SLOT_SLACK)
    }

    fn frame_interval(&self) -> Duration {
        let touching = self.last_touch.get().is_some_and(|t| t.elapsed() < TOUCH_FULL_RATE);
        let hz = self.refresh_hz.get();
        let hz = if touching { hz } else { hz.min(CALM_MAX_HZ) };
        Duration::from_secs_f32(1.0 / hz.max(1.0))
    }

    /// The system asks for the surface's content again.
    pub(crate) fn redraw_needed(&self) {
        self.force_next_frame.set(true);
        self.request_frame();
    }

    /// `InitWindow`: the activity has a (new) surface.
    pub(crate) fn surface_created(&self) {
        let Some(native) = self.app.native_window() else { return };
        let raw = RawWindow(native.clone());
        let instance = self.gpu_context.borrow().as_ref().map(|c| c.instance.clone());
        let replaced = match instance {
            Some(instance) => self.renderer.borrow_mut().replace_surface(&raw, surface_config(&native), &instance),
            None => Err(anyhow::anyhow!("GPU context is gone")),
        };
        if let Err(e) = replaced {
            log::error!("gpui-android: could not attach the new surface: {e:#}");
            match WgpuRenderer::new(self.gpu_context.clone(), &raw, surface_config(&native), None) {
                Ok(r) => *self.renderer.borrow_mut() = r,
                Err(e) => {
                    log::error!("gpui-android: could not recreate the renderer: {e:#}");
                    return;
                }
            }
        }
        *self.native.borrow_mut() = Some(raw);
        self.force_next_frame.set(true);
        self.set_visibility(WindowVisibility::Visible);
        self.geometry_changed();
    }

    /// `TerminateWindow`: the surface is about to be destroyed.
    pub(crate) fn surface_destroyed(&self) {
        self.renderer.borrow_mut().unconfigure_surface();
        *self.native.borrow_mut() = None;
        self.set_visibility(WindowVisibility::Hidden);
    }

    fn set_visibility(&self, v: WindowVisibility) {
        if self.visibility.replace(v) != v {
            self.visibility_callback.with(|cb| cb(v));
        }
    }

    pub(crate) fn set_active(&self, active: bool) {
        if self.active.replace(active) != active {
            if active { self.force_next_frame.set(true); }
            self.active_status_callback.with(|cb| cb(active));
        }
    }

    /// Size, density or orientation may have changed.
    pub(crate) fn geometry_changed(&self) {
        let Some(native) = self.native.borrow().as_ref().map(|r| r.0.clone()) else { return };
        let scale = density_scale(&self.app);
        let new_size = logical_size(&native, scale);
        let appearance = appearance(&self.app);
        if self.appearance.replace(appearance) != appearance {
            self.appearance_changed_callback.with(|cb| cb());
        }
        self.insets_changed();
        if new_size == self.size.get() && (scale - self.scale.get()).abs() < 0.001 {
            return;
        }
        self.size.set(new_size);
        self.scale.set(scale);
        self.renderer
            .borrow_mut()
            .update_drawable_size(size(DevicePixels(native.width()), DevicePixels(native.height())));
        self.force_next_frame.set(true);
        self.resize_callback.with(|cb| cb(new_size, scale));
    }

    pub(crate) fn insets_changed(&self) {
        let insets = crate::current_insets(self.scale.get());
        if *self.insets.borrow() != insets {
            *self.insets.borrow_mut() = insets.clone();
            self.insets_changed_callback.with(|cb| cb(insets));
        }
    }

    /// Draws if a frame is wanted and its slot has come.
    pub(crate) fn request_frame(&self) {
        // A background surface can remain attached while Android stops
        // consuming frames. Presenting then blocks the GPUI thread, starving
        // queue/preload and media-control tasks even though audio still runs.
        if !self.active.get() || !self.has_surface() || !self.frame_wait().is_zero() {
            return;
        }
        let force_render = self.force_next_frame.replace(false);
        if !force_render && !self.frame_wanted.get() {
            return;
        }
        // Cleared first: a draw that wants the next frame sets it again.
        self.frame_wanted.set(false);
        self.last_frame.set(Some(Instant::now()));
        self.request_frame_callback.with(|cb| {
            cb(RequestFrameOptions { force_render, require_presentation: force_render, ..Default::default() })
        });
        // An input got focus in this frame: now its text can be read.
        if std::mem::take(&mut self.ime.borrow_mut().pending) {
            input::push_ime_state(self);
            self.app.show_soft_input(true);
        }
    }

    /// Delivers one event; `true` when GPUI handled it.
    pub(crate) fn dispatch(&self, event: PlatformInput) -> bool {
        if matches!(event, PlatformInput::Touch(_) | PlatformInput::MouseDown(_) | PlatformInput::MouseMove(_) | PlatformInput::ScrollWheel(_)) {
            self.last_touch.set(Some(Instant::now()));
        }
        match &event {
            PlatformInput::Touch(t) => self.mouse_position.set(t.position),
            PlatformInput::MouseMove(m) => self.mouse_position.set(m.position),
            PlatformInput::MouseDown(m) => self.mouse_position.set(m.position),
            PlatformInput::MouseUp(m) => self.mouse_position.set(m.position),
            PlatformInput::KeyDown(k) => self.modifiers.set(k.keystroke.modifiers),
            _ => {}
        }
        self.input_callback
            .with(|cb| !cb(event).propagate)
            .unwrap_or(false)
    }

    /// Whether a system back press goes to GPUI.
    pub(crate) fn back_wanted(&self) -> bool {
        self.back_enabled.get()
    }

    /// System back (button or gesture). `false`: the app did not want it.
    pub(crate) fn back(&self) -> bool {
        if !self.back_enabled.get() {
            return false;
        }
        self.back_handler.with(|cb| cb()).is_some()
    }

    pub(crate) fn with_input_handler<R>(&self, f: impl FnOnce(&mut PlatformInputHandler) -> R) -> Option<R> {
        self.input_handler.with(f)
    }

    pub(crate) fn app(&self) -> &AndroidApp {
        &self.app
    }

    pub(crate) fn close(&self) {
        if let Some(cb) = self.close_callback.take() {
            cb();
        }
    }
}

impl HasWindowHandle for AndroidWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let native = self.0.native.borrow();
        let raw = native.as_ref().ok_or(HandleError::Unavailable)?;
        let handle = raw.window_handle()?.as_raw();
        // SAFETY: the native window outlives this borrow: it is only released
        // on `TerminateWindow`, on this thread, outside any GPUI call.
        Ok(unsafe { WindowHandle::borrow_raw(handle) })
    }
}

impl HasDisplayHandle for AndroidWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::android())
    }
}

#[allow(dead_code)]
fn _android_display_handle() -> AndroidDisplayHandle {
    AndroidDisplayHandle::new()
}

impl PlatformWindow for AndroidWindow {
    fn bounds(&self) -> Bounds<Pixels> {
        Bounds::new(Point::default(), self.0.size.get())
    }

    fn is_maximized(&self) -> bool {
        true
    }

    fn window_bounds(&self) -> WindowBounds {
        WindowBounds::Fullscreen(self.bounds())
    }

    fn content_size(&self) -> Size<Pixels> {
        self.0.size.get()
    }

    fn resize(&mut self, _size: Size<Pixels>) {}

    fn scale_factor(&self) -> f32 {
        self.0.scale.get()
    }

    fn appearance(&self) -> WindowAppearance {
        self.0.appearance.get()
    }

    fn display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        Some(Rc::new(self.0.display()))
    }

    fn mouse_position(&self) -> Point<Pixels> {
        self.0.mouse_position.get()
    }

    fn modifiers(&self) -> Modifiers {
        self.0.modifiers.get()
    }

    fn capslock(&self) -> Capslock {
        Capslock { on: false }
    }

    fn set_input_handler(&mut self, input_handler: PlatformInputHandler) {
        self.0.input_handler.set(input_handler);
    }

    fn take_input_handler(&mut self) -> Option<PlatformInputHandler> {
        self.0.input_handler.take()
    }

    fn set_text_input_configuration(&mut self, configuration: TextInputConfiguration) {
        let mut ty = InputType::TYPE_CLASS_TEXT;
        if !configuration.autocorrect {
            ty |= InputType::TYPE_TEXT_FLAG_NO_SUGGESTIONS;
        } else {
            ty |= InputType::TYPE_TEXT_FLAG_AUTO_CORRECT;
        }
        match configuration.autocapitalize {
            gpui::Autocapitalize::None => {}
            gpui::Autocapitalize::Words => ty |= InputType::TYPE_TEXT_FLAG_CAP_WORDS,
            gpui::Autocapitalize::Sentences => ty |= InputType::TYPE_TEXT_FLAG_CAP_SENTENCES,
            gpui::Autocapitalize::Characters => ty |= InputType::TYPE_TEXT_FLAG_CAP_CHARACTERS,
        }
        let action = match configuration.input_action {
            TextInputAction::Search => AndroidTextAction::Search,
            TextInputAction::Go => AndroidTextAction::Go,
            TextInputAction::Send => AndroidTextAction::Send,
            TextInputAction::Next => AndroidTextAction::Next,
            TextInputAction::Previous => AndroidTextAction::Previous,
            TextInputAction::Done => AndroidTextAction::Done,
            _ => AndroidTextAction::Done,
        };
        self.0.app.set_ime_editor_info(ty, action, ImeOptions::IME_FLAG_NO_FULLSCREEN);
    }

    fn prompt(
        &self,
        _level: PromptLevel,
        _msg: &str,
        _detail: Option<&str>,
        _answers: &[PromptButton],
    ) -> Option<futures::channel::oneshot::Receiver<usize>> {
        None
    }

    fn activate(&self) {}

    fn is_active(&self) -> bool {
        self.0.active.get()
    }

    fn visibility(&self) -> WindowVisibility {
        self.0.visibility.get()
    }

    fn is_hovered(&self) -> bool {
        false
    }

    fn background_appearance(&self) -> WindowBackgroundAppearance {
        WindowBackgroundAppearance::Opaque
    }

    fn set_title(&mut self, _title: &str) {}

    fn set_background_appearance(&self, _background_appearance: WindowBackgroundAppearance) {}

    fn minimize(&self) {
        java::move_task_to_back();
    }

    fn zoom(&self) {}

    fn toggle_fullscreen(&self) {}

    fn is_fullscreen(&self) -> bool {
        true
    }

    fn frame_waker(&self) -> Option<Rc<dyn Fn()>> {
        // Runs on the main thread, inside the run loop, which looks at the
        // flag before it sleeps again.
        let wanted = self.0.frame_wanted.clone();
        Some(Rc::new(move || wanted.set(true)))
    }

    fn schedule_frame(&self) {
        self.0.frame_wanted.set(true);
    }

    fn on_request_frame(&self, callback: Box<dyn FnMut(RequestFrameOptions)>) {
        self.0.request_frame_callback.set(callback);
    }

    fn on_input(&self, callback: Box<dyn FnMut(PlatformInput) -> DispatchEventResult>) {
        self.0.input_callback.set(callback);
    }

    fn on_active_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.0.active_status_callback.set(callback);
    }

    fn on_visibility_change(&self, callback: Box<dyn FnMut(WindowVisibility)>) {
        self.0.visibility_callback.set(callback);
    }

    fn on_hover_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.0.hover_status_callback.set(callback);
    }

    fn on_resize(&self, callback: Box<dyn FnMut(Size<Pixels>, f32)>) {
        self.0.resize_callback.set(callback);
    }

    fn on_moved(&self, callback: Box<dyn FnMut()>) {
        self.0.moved_callback.set(callback);
    }

    fn on_display_changed(&self, callback: Box<dyn FnMut()>) {
        self.0.display_changed_callback.set(callback);
    }

    fn on_should_close(&self, callback: Box<dyn FnMut() -> bool>) {
        self.0.should_close_callback.set(callback);
    }

    fn on_hit_test_window_control(&self, callback: Box<dyn FnMut() -> Option<WindowControlArea>>) {
        self.0.hit_test_callback.set(callback);
    }

    fn on_close(&self, callback: Box<dyn FnOnce()>) {
        self.0.close_callback.set(callback);
    }

    fn on_appearance_changed(&self, callback: Box<dyn FnMut()>) {
        self.0.appearance_changed_callback.set(callback);
    }

    fn draw(&self, scene: &Scene) {
        if self.0.native.borrow().is_some() {
            self.0.renderer.borrow_mut().draw(scene);
        }
    }

    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.0.renderer.borrow().sprite_atlas().clone()
    }

    fn is_subpixel_rendering_supported(&self) -> bool {
        false
    }

    fn gpu_specs(&self) -> Option<GpuSpecs> {
        self.0.renderer.borrow().gpu_specs()
    }

    fn update_ime_position(&self, _bounds: Bounds<Pixels>) {}

    fn insets(&self) -> WindowInsets {
        self.0.insets.borrow().clone()
    }

    fn on_insets_changed(&self, callback: Box<dyn FnMut(WindowInsets)>) {
        self.0.insets_changed_callback.set(callback);
    }

    fn set_back_handler(&self, callback: Box<dyn FnMut()>) {
        self.0.back_handler.set(callback);
    }

    fn set_back_enabled(&self, enabled: bool) {
        self.0.back_enabled.set(enabled);
    }

    fn show_soft_keyboard(&self) {
        self.0.app.show_soft_input(true);
    }

    fn hide_soft_keyboard(&self) {
        self.0.app.hide_soft_input(false);
    }

    fn text_input_state_changed(&self, change: TextInputStateChange) {
        match change {
            // Sent from inside the draw, where the input cannot be read yet
            // (it would push an empty text): see `request_frame`.
            TextInputStateChange::FocusGained => {
                self.0.ime.borrow_mut().pending = true;
                self.0.frame_wanted.set(true);
            }
            TextInputStateChange::FocusLost => {
                self.0.ime.borrow_mut().reset();
                self.0.app.hide_soft_input(false);
            }
            TextInputStateChange::SelectionChanged | TextInputStateChange::ContentChanged => {
                if !self.0.ime.borrow().applying {
                    input::push_ime_state(&self.0);
                }
            }
        }
    }
}

impl Drop for AndroidWindow {
    fn drop(&mut self) {
        self.0.renderer.borrow_mut().destroy();
    }
}

/// Edges in device pixels to logical ones.
pub(crate) fn edges(top: i32, right: i32, bottom: i32, left: i32, scale: f32) -> Edges<Pixels> {
    Edges {
        top: px(top.max(0) as f32 / scale),
        right: px(right.max(0) as f32 / scale),
        bottom: px(bottom.max(0) as f32 / scale),
        left: px(left.max(0) as f32 / scale),
    }
}
