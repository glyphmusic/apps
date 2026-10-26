//! Device connectivity, independent of the availability of the Glyph server.
use std::sync::atomic::{AtomicU8, Ordering};

// Unknown until Android reports the capabilities of its default network.
static PLATFORM: AtomicU8 = AtomicU8::new(0);

pub fn set_platform_online(online: bool) {
    PLATFORM.store(if online { 1 } else { 2 }, Ordering::Release);
}

pub fn platform_online() -> Option<bool> {
    match PLATFORM.load(Ordering::Acquire) {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}
