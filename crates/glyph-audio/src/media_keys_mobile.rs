//! Mobile stand-in for the souvlaki media keys: same API, never available.
//! The app shell drives its own MediaSession (Android) / MPRemoteCommandCenter
//! (iOS) and calls the player directly.

use crate::MediaKeyEvent;
use anyhow::{anyhow, Result};
use glyph_protocol::Track;
use std::ffi::c_void;
use std::time::Duration;

pub struct MediaKeys {
    events: flume::Receiver<MediaKeyEvent>,
}

impl MediaKeys {
    pub fn new(_hwnd: Option<*mut c_void>) -> Result<Self> {
        Err(anyhow!("media keys are handled by the mobile app shell"))
    }

    pub fn events(&self) -> flume::Receiver<MediaKeyEvent> {
        self.events.clone()
    }

    pub fn set_now_playing(&self, _track: Option<&Track>, _playing: bool, _position: Duration) {}
}
