//! GPUI platform for Android, on top of `android-activity` (GameActivity) and
//! the shared wgpu renderer (Vulkan, GLES fallback).
//!
//! Upstream GPUI ships iOS, macOS, Windows, Linux and web backends but no
//! Android one. This crate fills that gap with the same shape as
//! `gpui_ios`: one full-screen window bound to the activity's surface, raw
//! touches handed to GPUI's portable gesture recognizers, safe-area and
//! keyboard insets, the system back button, app lifecycle and the soft
//! keyboard through GameActivity's text input state.
//!
//! The Java side (`MainActivity` in the app's Gradle project) reports window
//! insets through [`set_system_insets`] and provides a few helpers that are
//! called over JNI (see [`java`]).
#![cfg(target_os = "android")]

mod dispatcher;
mod display;
mod input;
pub mod java;
mod platform;
mod window;

pub use android_activity;
pub use platform::AndroidPlatform;

use android_activity::{AndroidApp, AndroidAppWaker};
use gpui::WindowInsets;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

/// A GPUI application running on this activity. Call from `android_main`.
pub fn application(app: AndroidApp) -> gpui::Application {
    gpui::Application::with_platform(Rc::new(AndroidPlatform::new(app)))
}

pub(crate) struct CallbackSlot<T> {
    value: RefCell<Option<T>>,
    generation: Cell<u64>,
}

impl<T> Default for CallbackSlot<T> {
    fn default() -> Self {
        Self { value: RefCell::new(None), generation: Cell::new(0) }
    }
}

impl<T> CallbackSlot<T> {
    pub(crate) fn set(&self, value: T) {
        self.generation.set(self.generation.get().wrapping_add(1));
        drop(self.value.replace(Some(value)));
    }

    pub(crate) fn take(&self) -> Option<T> {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.value.borrow_mut().take()
    }

    /// Runs `callback` with the value taken out, so a callback that re-enters
    /// the platform cannot alias it. A value replaced meanwhile wins.
    pub(crate) fn with<R>(&self, callback: impl FnOnce(&mut T) -> R) -> Option<R> {
        let mut value = self.value.borrow_mut().take()?;
        let generation = self.generation.get();
        let result = callback(&mut value);
        if self.generation.get() == generation {
            drop(self.value.replace(Some(value)));
        }
        Some(result)
    }
}

// Insets in device pixels, written by the Java UI thread.
// [safe top, right, bottom, left, ime top, right, bottom, left]
static INSETS: [AtomicI32; 8] = [const { AtomicI32::new(0) }; 8];
static INSETS_DIRTY: AtomicBool = AtomicBool::new(false);
static WAKER: OnceLock<AndroidAppWaker> = OnceLock::new();

/// Window insets from `View.onApplyWindowInsets`, in device pixels:
/// `safe` = system bars + display cutout, `ime` = the soft keyboard.
/// Each is `[top, right, bottom, left]`. Callable from any thread.
pub fn set_system_insets(safe: [i32; 4], ime: [i32; 4]) {
    for (slot, value) in INSETS.iter().zip(safe.into_iter().chain(ime)) {
        slot.store(value, Ordering::Relaxed);
    }
    INSETS_DIRTY.store(true, Ordering::Release);
    if let Some(waker) = WAKER.get() {
        waker.wake();
    }
}

pub(crate) fn take_insets_dirty() -> bool {
    INSETS_DIRTY.swap(false, Ordering::Acquire)
}

pub(crate) fn current_insets(scale: f32) -> WindowInsets {
    let v = |i: usize| INSETS[i].load(Ordering::Relaxed);
    WindowInsets {
        safe_area: window::edges(v(0), v(1), v(2), v(3), scale),
        ime: window::edges(v(4), v(5), v(6), v(7), scale),
    }
}

pub(crate) fn set_waker(waker: AndroidAppWaker) {
    let _ = WAKER.set(waker);
}

/// Wakes the run loop from any thread (e.g. a JNI callback).
pub fn wake() {
    if let Some(waker) = WAKER.get() {
        waker.wake();
    }
}
