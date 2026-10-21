//! Media keys on phones. Same API as the desktop `MediaKeys` (souvlaki); the
//! native app shell registers a [`NowPlayingSink`] (notification + lock screen)
//! and feeds button presses back through [`push_event`].

use crate::MediaKeyEvent;
use anyhow::Result;
use glyph_protocol::Track;
use std::ffi::c_void;
use std::sync::Mutex;
use std::time::Duration;

/// `(track, playing, position)`; `None` track: nothing loaded.
pub type NowPlayingSink = Box<dyn Fn(Option<&Track>, bool, Duration) + Send>;

static SINK: Mutex<Option<NowPlayingSink>> = Mutex::new(None);
static EVENTS: Mutex<Option<flume::Sender<MediaKeyEvent>>> = Mutex::new(None);

/// Called once by the app shell.
pub fn set_now_playing_sink(sink: NowPlayingSink) {
    *SINK.lock().unwrap() = Some(sink);
}

/// A media button from the shell (notification, lock screen, headset).
pub fn push_event(ev: MediaKeyEvent) {
    if let Some(tx) = EVENTS.lock().unwrap().as_ref() {
        let _ = tx.send(ev);
    }
}

pub struct MediaKeys {
    events: flume::Receiver<MediaKeyEvent>,
}

impl MediaKeys {
    pub fn new(_hwnd: Option<*mut c_void>) -> Result<Self> {
        let (tx, events) = flume::unbounded();
        *EVENTS.lock().unwrap() = Some(tx);
        Ok(Self { events })
    }

    pub fn events(&self) -> flume::Receiver<MediaKeyEvent> {
        self.events.clone()
    }

    pub fn set_now_playing(&self, track: Option<&Track>, playing: bool, position: Duration) {
        if let Some(sink) = SINK.lock().unwrap().as_ref() {
            sink(track, playing, position);
        }
    }
}
