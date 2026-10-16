//! Thin wrapper around glyph-audio's embed host (official YouTube /
//! SoundCloud player as a child webview in place of the Now Playing cover).
//!
//! Call order: `attach` once after the window exists and the player is ready
//! (UI thread) → `set_bounds` whenever the cover slot is laid out →
//! `set_visible` (only in Now Playing while an embed plays) → `pump` every
//! frame (~16 ms) on the UI thread.

use crate::guard::guard;
use glyph_audio::PlayerController;
use raw_window_handle::RawWindowHandle;

#[derive(Default)]
pub struct EmbedHost {
    parent: Option<RawWindowHandle>,
    attached: bool,
    failed: bool,
    bounds: Option<(f32, f32, f32, f32)>,
    visible: bool,
}

impl EmbedHost {
    pub fn new(parent: Option<RawWindowHandle>) -> Self {
        Self { parent, ..Default::default() }
    }

    pub fn attach(&mut self, player: &PlayerController) {
        if self.attached || self.failed {
            return;
        }
        let Some(parent) = self.parent else {
            self.failed = true;
            return;
        };
        match guard(|| player.attach_embed_host(parent)) {
            Some(Ok(())) => self.attached = true,
            Some(Err(e)) => {
                tracing::warn!("embed host unavailable, using popup/browser fallback: {e:#}");
                self.failed = true;
            }
            None => self.failed = true,
        }
    }

    pub fn set_bounds(&mut self, player: &PlayerController, b: (f32, f32, f32, f32)) {
        if !self.attached || self.bounds == Some(b) {
            return;
        }
        self.bounds = Some(b);
        guard(|| player.set_embed_bounds(b.0, b.1, b.2, b.3));
    }

    pub fn set_visible(&mut self, player: &PlayerController, visible: bool) {
        if !self.attached || self.visible == visible {
            return;
        }
        self.visible = visible;
        guard(|| player.set_embed_visible(visible));
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn pump(&mut self, player: &PlayerController) {
        if self.attached {
            guard(|| player.pump_embed());
        }
    }
}
