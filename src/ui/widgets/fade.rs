//! Soft cross-fade when a region switches content (page, settings page,
//! side panel): the new content fades in and settles from a few pixels
//! below, so a switch never feels like a hard cut.

use super::*;
use std::hash::{Hash, Hasher};

/// Fade length. Short on purpose: it should be felt, not waited for.
pub const FADE_MS: u64 = 190;
/// How far new content rises while it fades in.
const RISE: f32 = 6.0;

/// Stable key for whatever a region currently shows; pass it to [`fade_in`].
pub fn fade_key(parts: impl Hash) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}

/// Wraps `child` so it fades in whenever `key` changes. `slot` names the
/// region; the same slot with the same key keeps the content as it is.
/// `fill`: the wrapper takes its parent's full size (pages that size
/// themselves to the frame); otherwise only the width.
pub fn fade_in(slot: &'static str, key: u64, fill: bool, child: impl IntoElement) -> AnyElement {
    // A new element id per key, so the animation runs once per switch.
    div()
        .relative()
        .w_full()
        .when(fill, |d| d.h_full())
        .child(child)
        .with_animation(
            ElementId::NamedInteger(slot.into(), key),
            gpui::Animation::new(Duration::from_millis(FADE_MS)),
            |d, t| {
                // Ease-out cubic: most of the fade early, a soft landing.
                let e = 1.0 - (1.0 - t).powi(3);
                d.opacity(e).top(px(RISE * (1.0 - e)))
            },
        )
        .into_any_element()
}
