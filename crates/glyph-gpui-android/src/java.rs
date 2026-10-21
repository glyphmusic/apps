//! Calls into the hosting activity over JNI.
//!
//! The app's `MainActivity` implements the `gpui*` helpers; anything that
//! needs the Android SDK (intents, the clipboard service, the display) lives
//! there rather than in hand-written JNI.

use jni::objects::{JObject, JString};
use jni::{Env, JValue, JavaVM, jni_sig, jni_str};
use std::sync::atomic::{AtomicUsize, Ordering};

// The current activity (a global reference owned by android-activity for the
// activity's lifetime). ndk-context only holds the Application, which does
// not have the `gpui*` helpers.
static ACTIVITY: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn set_activity(activity: *mut std::ffi::c_void) {
    ACTIVITY.store(activity as usize, Ordering::Release);
}

/// Runs `f` with the activity on this thread (attached on demand). Logs and
/// returns `None` on any JNI error or Java exception.
pub fn with_activity<T>(
    what: &str,
    f: impl FnOnce(&mut Env, &JObject) -> jni::errors::Result<T>,
) -> Option<T> {
    let ctx = ndk_context::android_context();
    let activity_ptr = ACTIVITY.load(Ordering::Acquire);
    if ctx.vm().is_null() || activity_ptr == 0 {
        return None;
    }
    // SAFETY: ndk-context holds the process's JavaVM.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
    let result = vm.attach_current_thread(|env| -> jni::errors::Result<T> {
        // SAFETY: a global reference stays valid on every thread; `JObject`
        // never deletes the reference it wraps.
        let activity = unsafe { JObject::from_raw(env, activity_ptr as jni::sys::jobject) };
        let result = f(env, &activity);
        if env.exception_check() {
            env.exception_clear();
        }
        result
    });
    result.map_err(|e| log::warn!("gpui-android: {what} failed: {e}")).ok()
}

pub fn open_url(url: &str) {
    with_activity("open url", |env, activity| {
        let url = env.new_string(url)?;
        env.call_method(activity, jni_str!("gpuiOpenUrl"), jni_sig!("(Ljava/lang/String;)V"), &[
            JValue::Object(&url),
        ])?;
        Ok(())
    });
}

pub fn set_clipboard(text: &str) {
    with_activity("set clipboard", |env, activity| {
        let text = env.new_string(text)?;
        env.call_method(activity, jni_str!("gpuiSetClipboard"), jni_sig!("(Ljava/lang/String;)V"), &[
            JValue::Object(&text),
        ])?;
        Ok(())
    });
}

pub fn clipboard() -> Option<String> {
    with_activity("read clipboard", |env, activity| {
        let value = env
            .call_method(activity, jni_str!("gpuiGetClipboard"), jni_sig!("()Ljava/lang/String;"), &[])?
            .l()?;
        if value.is_null() {
            return Ok(None);
        }
        let value = env.cast_local::<JString>(value)?;
        Ok(Some(value.try_to_string(env)?))
    })
    .flatten()
}

/// `Activity.moveTaskToBack`: like pressing home, the app keeps running
/// (and playing).
pub fn move_task_to_back() {
    with_activity("move task to back", |env, activity| {
        env.call_method(activity, jni_str!("moveTaskToBack"), jni_sig!("(Z)Z"), &[JValue::Bool(true.into())])?;
        Ok(())
    });
}

/// The display's refresh rate in Hz.
pub fn refresh_rate() -> Option<f32> {
    with_activity("read refresh rate", |env, activity| {
        env.call_method(activity, jni_str!("gpuiRefreshRate"), jni_sig!("()F"), &[])?.f()
    })
    .filter(|hz| hz.is_finite() && *hz >= 1.0)
}
