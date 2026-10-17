//! Guards around calls into glyph-core / glyph-audio.
//!
//! Those crates are developed in parallel; while parts of them are still
//! `todo!()`, a panic must not take the UI down. Every call from the UI goes
//! through [`guard`], which turns a panic into `None` and logs it once per
//! call site.

use std::collections::HashSet;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Mutex;

static SEEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Installs a panic hook that reports each distinct panic location only once
/// (unimplemented backend calls would otherwise flood the log 30×/s).
pub fn install_panic_hook() {
    let default = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let loc = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        let soft = msg.contains("not yet implemented") || msg.contains("not implemented");
        if soft {
            let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
            if seen.get_or_insert_with(HashSet::new).insert(loc.clone()) {
                tracing::warn!("backend not ready: {msg} at {loc}");
            }
        } else {
            // Leave a trace for crashes of the windowed app (no console there).
            let bt = std::backtrace::Backtrace::force_capture();
            let log = glyph_core::paths::data_dir().join("crash.log");
            let _ = std::fs::write(&log, format!("{msg}\nat {loc}\n\n{bt}\n"));
            default(info);
        }
    }));
}

/// Runs `f`, returning `None` if it panicked.
pub fn guard<T>(f: impl FnOnce() -> T) -> Option<T> {
    panic::catch_unwind(AssertUnwindSafe(f)).ok()
}

/// Same as [`guard`] for `anyhow::Result`, flattening both failure kinds into a message.
pub fn guard_res<T>(f: impl FnOnce() -> anyhow::Result<T>) -> Result<T, String> {
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(format!("{e:#}")),
        Err(_) => Err("not implemented".into()),
    }
}

/// Awaits a future, turning a panic into an error message.
pub async fn guard_async<T>(
    fut: impl std::future::Future<Output = anyhow::Result<T>>,
) -> Result<T, String> {
    use futures::FutureExt;
    match AssertUnwindSafe(fut).catch_unwind().await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(format!("{e:#}")),
        Err(_) => Err("not implemented".into()),
    }
}
