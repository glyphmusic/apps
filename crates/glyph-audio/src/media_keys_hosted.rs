//! Media keys on phones. Same API as the desktop `MediaKeys` (souvlaki); the
//! native app shell registers a [`NowPlayingSink`] (notification + lock screen)
//! and feeds button presses back through [`push_event`].

use crate::MediaKeyEvent;
use anyhow::Result;
use glyph_protocol::Track;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// What the shell shows.
pub struct NowPlaying<'a> {
    /// `None`: nothing loaded.
    pub track: Option<&'a Track>,
    pub playing: bool,
    pub position: Duration,
    /// Cached cover file (the notification's artwork).
    pub cover: Option<&'a Path>,
    /// The song is in Liked Songs (the add button shows a check).
    pub liked: bool,
}

pub type NowPlayingSink = Box<dyn Fn(&NowPlaying<'_>) + Send>;

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

#[derive(Default)]
struct Shown {
    track: Option<Track>,
    playing: bool,
    position: Duration,
    cover: Option<PathBuf>,
    liked: bool,
}

pub struct MediaKeys {
    events: flume::Receiver<MediaKeyEvent>,
    /// Last state handed to the shell; `set_artwork` resends it.
    shown: Mutex<Shown>,
}

impl MediaKeys {
    pub fn new(_hwnd: Option<*mut c_void>) -> Result<Self> {
        let (tx, events) = flume::unbounded();
        *EVENTS.lock().unwrap() = Some(tx);
        Ok(Self { events, shown: Mutex::default() })
    }

    pub fn events(&self) -> flume::Receiver<MediaKeyEvent> {
        self.events.clone()
    }

    pub fn set_now_playing(&self, track: Option<&Track>, playing: bool, position: Duration) {
        let mut shown = self.shown.lock().unwrap();
        shown.track = track.cloned();
        shown.playing = playing;
        shown.position = position;
        send(&shown);
    }

    /// Cover file and like state of the current song; sent when they change.
    pub fn set_artwork(&self, cover: Option<&Path>, liked: bool) {
        let mut shown = self.shown.lock().unwrap();
        if shown.cover.as_deref() == cover && shown.liked == liked {
            return;
        }
        shown.cover = cover.map(Path::to_path_buf);
        shown.liked = liked;
        if shown.track.is_some() {
            send(&shown);
        }
    }
}

fn send(shown: &Shown) {
    if let Some(sink) = SINK.lock().unwrap().as_ref() {
        sink(&NowPlaying {
            track: shown.track.as_ref(),
            playing: shown.playing,
            position: shown.position,
            cover: shown.cover.as_deref(),
            liked: shown.liked,
        });
    }
}
