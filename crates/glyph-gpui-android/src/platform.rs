//! `Platform` for Android: one activity, one full-screen window, the run
//! loop on the `android_main` thread.
//!
//! The loop waits on the activity's looper until the window's next frame
//! slot while GPUI wants a frame, otherwise until an event or a wake (main-
//! thread task, media key); then it drains GPUI's main-thread tasks,
//! delivers input and draws if a frame is due. An idle app does not wake.
//! GPUI's `on_finish_launching` runs once the first surface exists, because
//! `open_window` needs it.

use crate::dispatcher::AndroidDispatcher;
use crate::window::{AndroidWindow, AndroidWindowState};
use crate::{CallbackSlot, input, java};
use crate::queue::Receiver;
use android_activity::{AndroidApp, InputStatus, MainEvent, PollEvent};
use anyhow::anyhow;
use futures::channel::oneshot;
use gpui::{
    Action, AnyWindowHandle, AppLifecyclePhase, BackgroundExecutor, ClipboardItem, CursorStyle,
    DummyKeyboardMapper, ForegroundExecutor, Keymap, Menu, MenuItem, PathPromptOptions, Platform,
    PlatformDisplay, PlatformKeyboardLayout, PlatformKeyboardMapper, PlatformTextSystem,
    PlatformWindow, Result, RunnableVariant, Task, ThermalState,
    WindowAppearance, WindowParams,
};
use gpui_wgpu::{CosmicTextSystem, GpuContext};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::Arc;

pub struct AndroidPlatform(Rc<AndroidPlatformState>);

pub(crate) struct AndroidPlatformState {
    app: AndroidApp,
    background_executor: BackgroundExecutor,
    foreground_executor: ForegroundExecutor,
    main_receiver: Receiver<RunnableVariant>,
    text_system: Arc<dyn PlatformTextSystem>,
    gpu_context: GpuContext,
    window: RefCell<Weak<AndroidWindowState>>,
    quit_requested: Cell<bool>,
    open_urls: CallbackSlot<Box<dyn FnMut(Vec<String>)>>,
    quit: CallbackSlot<Box<dyn FnMut() -> bool>>,
    lifecycle: CallbackSlot<Box<dyn FnMut(AppLifecyclePhase)>>,
    memory_warning: CallbackSlot<Box<dyn FnMut()>>,
}

impl AndroidPlatform {
    /// Call on the `android_main` thread.
    pub fn new(app: AndroidApp) -> Self {
        java::set_activity(app.activity_as_ptr());
        let waker = app.create_waker();
        crate::set_waker(waker.clone());
        let (dispatcher, main_receiver) = AndroidDispatcher::new(waker);
        let dispatcher = Arc::new(dispatcher);
        Self(Rc::new(AndroidPlatformState {
            app,
            background_executor: BackgroundExecutor::new(dispatcher.clone()),
            foreground_executor: ForegroundExecutor::new(dispatcher),
            main_receiver,
            // Android ships Roboto; the app registers its own fonts on top.
            text_system: Arc::new(CosmicTextSystem::new("Roboto")),
            gpu_context: Rc::new(RefCell::new(None)),
            window: RefCell::new(Weak::new()),
            quit_requested: Cell::new(false),
            open_urls: CallbackSlot::default(),
            quit: CallbackSlot::default(),
            lifecycle: CallbackSlot::default(),
            memory_warning: CallbackSlot::default(),
        }))
    }

    /// Deep links / shared links from the activity's intents (called from JNI
    /// glue through [`crate::wake`]-safe queues by the app).
    pub fn open_urls(&self, urls: Vec<String>) {
        self.0.open_urls.with(|cb| cb(urls));
    }
}

impl AndroidPlatformState {
    fn window(&self) -> Option<Rc<AndroidWindowState>> {
        self.window.borrow().upgrade()
    }

    fn lifecycle(&self, phase: AppLifecyclePhase) {
        self.lifecycle.with(|cb| cb(phase));
    }

    fn run_main_tasks(&self) {
        // Bounded, so a task that keeps re-queueing itself cannot starve input.
        for _ in 0..1024 {
            match self.main_receiver.try_pop() {
                Some(runnable) => {
                    runnable.run();
                }
                None => break,
            }
        }
    }

    fn handle_main_event(&self, event: MainEvent, launched: &mut bool) {
        match event {
            MainEvent::InitWindow { .. } => {
                if let Some(w) = self.window() {
                    w.surface_created();
                }
                if !*launched {
                    // `run` opens the GPUI window right after this.
                    *launched = true;
                }
            }
            MainEvent::TerminateWindow { .. } => {
                if let Some(w) = self.window() {
                    w.surface_destroyed();
                }
            }
            MainEvent::WindowResized { .. } | MainEvent::ContentRectChanged { .. } | MainEvent::ConfigChanged { .. } => {
                if let Some(w) = self.window() {
                    w.geometry_changed();
                }
            }
            MainEvent::InsetsChanged { .. } => {
                if let Some(w) = self.window() {
                    w.insets_changed();
                }
            }
            MainEvent::RedrawNeeded { .. } => {
                if let Some(w) = self.window() {
                    w.redraw_needed();
                }
            }
            MainEvent::GainedFocus => {
                if let Some(w) = self.window() {
                    w.set_active(true);
                }
                self.lifecycle(AppLifecyclePhase::Active);
            }
            MainEvent::LostFocus => {
                if let Some(w) = self.window() {
                    w.set_active(false);
                }
                self.lifecycle(AppLifecyclePhase::Inactive);
            }
            MainEvent::Start => self.lifecycle(AppLifecyclePhase::Foreground),
            MainEvent::Resume { .. } => self.lifecycle(AppLifecyclePhase::Foreground),
            MainEvent::Pause => self.lifecycle(AppLifecyclePhase::Inactive),
            MainEvent::Stop => self.lifecycle(AppLifecyclePhase::Background),
            MainEvent::LowMemory => {
                self.memory_warning.with(|cb| cb());
            }
            MainEvent::Destroy => {
                self.lifecycle(AppLifecyclePhase::Disconnected);
                self.quit.with(|cb| cb());
                if let Some(w) = self.window() {
                    w.close();
                }
                self.quit_requested.set(true);
            }
            _ => {}
        }
    }

    fn handle_input(&self) {
        let Some(w) = self.window() else { return };
        let Ok(mut events) = self.app.input_events_iter() else { return };
        loop {
            let more = events.next(|event| {
                if input::handle(&w, event) { InputStatus::Handled } else { InputStatus::Unhandled }
            });
            if !more {
                break;
            }
        }
        input::flush_motions(&w);
    }
}

struct AndroidKeyboardLayout;

impl PlatformKeyboardLayout for AndroidKeyboardLayout {
    fn id(&self) -> &str {
        "android-default"
    }

    fn name(&self) -> &str {
        "Android"
    }
}

fn not_supported<T: Send + 'static>(what: &str) -> oneshot::Receiver<Result<T>> {
    let (tx, rx) = oneshot::channel();
    let _ = tx.send(Err(anyhow!("{what} is not available on Android yet")));
    rx
}

impl Platform for AndroidPlatform {
    fn background_executor(&self) -> BackgroundExecutor {
        self.0.background_executor.clone()
    }

    fn foreground_executor(&self) -> ForegroundExecutor {
        self.0.foreground_executor.clone()
    }

    fn text_system(&self) -> Arc<dyn PlatformTextSystem> {
        self.0.text_system.clone()
    }

    fn gestures(&self) -> Option<Rc<dyn gpui::PlatformGestures>> {
        Some(Rc::new(input::AndroidGestures))
    }

    fn run(&self, on_finish_launching: Box<dyn 'static + FnOnce()>) {
        let state = self.0.clone();
        let app = state.app.clone();
        let mut on_finish_launching = Some(on_finish_launching);
        let mut has_surface = false;
        while !state.quit_requested.get() {
            let timeout = state.window().and_then(|w| w.next_frame_in());
            app.poll_events(timeout, |event| {
                if let PollEvent::Main(main) = event {
                    state.handle_main_event(main, &mut has_surface);
                }
            });
            if has_surface {
                if let Some(launch) = on_finish_launching.take() {
                    launch();
                }
            }
            state.run_main_tasks();
            state.handle_input();
            if let Some(w) = state.window() {
                if crate::take_insets_dirty() {
                    w.insets_changed();
                }
                w.request_frame();
            }
        }
    }

    fn quit(&self) {
        // Android apps do not quit themselves; going to the background keeps
        // playback running.
        java::move_task_to_back();
    }

    fn restart(&self, _binary_path: Option<PathBuf>, _arguments: Vec<std::ffi::OsString>) {
        log::warn!("gpui-android: apps cannot restart themselves");
    }

    fn activate(&self, _ignoring_other_apps: bool) {}

    fn hide(&self) {
        java::move_task_to_back();
    }

    fn hide_other_apps(&self) {}

    fn unhide_other_apps(&self) {}

    fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>> {
        self.primary_display().into_iter().collect()
    }

    fn primary_display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        self.0.window().map(|w| Rc::new(w.display()) as Rc<dyn PlatformDisplay>)
    }

    fn on_displays_changed(&self, _callback: Box<dyn FnMut()>) {}

    fn active_window(&self) -> Option<AnyWindowHandle> {
        None
    }

    fn open_window(&self, handle: AnyWindowHandle, options: WindowParams) -> anyhow::Result<Box<dyn PlatformWindow>> {
        let window = AndroidWindow::new(handle, options, &self.0.app, self.0.gpu_context.clone())?;
        *self.0.window.borrow_mut() = Rc::downgrade(&window.0);
        Ok(Box::new(window))
    }

    fn window_appearance(&self) -> WindowAppearance {
        match self.0.app.config().ui_mode_night() {
            ndk::configuration::UiModeNight::Yes => WindowAppearance::Dark,
            _ => WindowAppearance::Light,
        }
    }

    fn open_url(&self, url: &str) {
        java::open_url(url);
    }

    fn on_open_urls(&self, callback: Box<dyn FnMut(Vec<String>)>) {
        self.0.open_urls.set(callback);
    }

    fn register_url_scheme(&self, _url: &str) -> Task<Result<()>> {
        // Declared as an intent filter in the manifest.
        Task::ready(Ok(()))
    }

    fn prompt_for_paths(&self, _options: PathPromptOptions) -> oneshot::Receiver<Result<Option<Vec<PathBuf>>>> {
        not_supported("the file picker")
    }

    fn prompt_for_new_path(&self, _directory: &Path, _suggested_name: Option<&str>) -> oneshot::Receiver<Result<Option<PathBuf>>> {
        not_supported("the save dialog")
    }

    fn can_select_mixed_files_and_dirs(&self) -> bool {
        false
    }

    fn reveal_path(&self, _path: &Path) {}

    fn open_with_system(&self, _path: &Path) {}

    fn on_quit(&self, callback: Box<dyn FnMut() -> bool>) {
        self.0.quit.set(callback);
    }

    fn on_reopen(&self, _callback: Box<dyn FnMut()>) {}

    fn on_system_sleep(&self, _callback: Box<dyn FnMut()>) {}

    fn on_system_wake(&self, _callback: Box<dyn FnMut()>) {}

    fn on_app_lifecycle(&self, callback: Box<dyn FnMut(AppLifecyclePhase)>) {
        self.0.lifecycle.set(callback);
    }

    fn on_memory_warning(&self, callback: Box<dyn FnMut()>) {
        self.0.memory_warning.set(callback);
    }

    fn set_menus(&self, _menus: Vec<Menu>, _keymap: &Keymap) {}

    fn set_dock_menu(&self, _menu: Vec<MenuItem>, _keymap: &Keymap) {}

    fn on_app_menu_action(&self, _callback: Box<dyn FnMut(&dyn Action)>) {}

    fn on_will_open_app_menu(&self, _callback: Box<dyn FnMut()>) {}

    fn on_validate_app_menu_command(&self, _callback: Box<dyn FnMut(&dyn Action) -> bool>) {}

    fn thermal_state(&self) -> ThermalState {
        ThermalState::Nominal
    }

    fn on_thermal_state_change(&self, _callback: Box<dyn FnMut()>) {}

    fn prevent_idle_sleep(&self, _reason: &str) -> Task<Result<gpui::ActivityGuard>> {
        Task::ready(Err(anyhow!("idle sleep prevention is not available on Android yet")))
    }

    fn app_path(&self) -> Result<PathBuf> {
        self.0.app.internal_data_path().ok_or_else(|| anyhow!("no app data path"))
    }

    fn path_for_auxiliary_executable(&self, name: &str) -> Result<PathBuf> {
        Err(anyhow!("no auxiliary executables on Android ({name})"))
    }

    fn set_cursor_style(&self, _style: CursorStyle) {}

    fn hide_cursor_until_mouse_moves(&self) {}

    fn is_cursor_visible(&self) -> bool {
        false
    }

    fn should_auto_hide_scrollbars(&self) -> bool {
        true
    }

    fn read_from_clipboard(&self) -> Option<ClipboardItem> {
        java::clipboard().map(ClipboardItem::new_string)
    }

    fn write_to_clipboard(&self, item: ClipboardItem) {
        if let Some(text) = item.text() {
            java::set_clipboard(&text);
        }
    }

    fn write_credentials(&self, _url: &str, _username: &str, _password: &[u8]) -> Task<Result<()>> {
        Task::ready(Err(anyhow!("credentials are stored by the app on Android")))
    }

    fn read_credentials(&self, _url: &str) -> Task<Result<Option<(String, Vec<u8>)>>> {
        Task::ready(Ok(None))
    }

    fn delete_credentials(&self, _url: &str) -> Task<Result<()>> {
        Task::ready(Ok(()))
    }

    fn keyboard_layout(&self) -> Box<dyn PlatformKeyboardLayout> {
        Box::new(AndroidKeyboardLayout)
    }

    fn keyboard_mapper(&self) -> Rc<dyn PlatformKeyboardMapper> {
        Rc::new(DummyKeyboardMapper)
    }

    fn on_keyboard_layout_change(&self, _callback: Box<dyn FnMut()>) {}
}
