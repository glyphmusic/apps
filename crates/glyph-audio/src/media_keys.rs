//! OS media keys + now-playing info through souvlaki (SMTC on Windows, MPRIS
//! on Linux, MPRemoteCommandCenter on macOS). The souvlaki handle lives on its
//! own thread (it is not `Send`/`Sync` everywhere); the public type is a
//! cheap channel handle.

use crate::MediaKeyEvent;
use anyhow::{anyhow, Result};
use glyph_protocol::Track;
use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig, SeekDirection};
use std::ffi::c_void;
use std::time::Duration;

enum Msg {
    Now { track: Option<Track>, playing: bool, position: Duration },
    Quit,
}

pub struct MediaKeys {
    tx: flume::Sender<Msg>,
    events: flume::Receiver<MediaKeyEvent>,
}

pub(crate) fn map_event(e: &MediaControlEvent) -> Option<MediaKeyEvent> {
    Some(match e {
        MediaControlEvent::Play => MediaKeyEvent::Play,
        MediaControlEvent::Pause => MediaKeyEvent::Pause,
        MediaControlEvent::Toggle => MediaKeyEvent::Toggle,
        MediaControlEvent::Next => MediaKeyEvent::Next,
        MediaControlEvent::Previous => MediaKeyEvent::Previous,
        MediaControlEvent::Stop => MediaKeyEvent::Stop,
        MediaControlEvent::Seek(SeekDirection::Forward) | MediaControlEvent::SeekBy(SeekDirection::Forward, _) => {
            MediaKeyEvent::SeekForward
        }
        MediaControlEvent::Seek(SeekDirection::Backward) | MediaControlEvent::SeekBy(SeekDirection::Backward, _) => {
            MediaKeyEvent::SeekBackward
        }
        _ => return None,
    })
}

impl MediaKeys {
    /// `hwnd`: the main window handle on Windows, ignored elsewhere.
    pub fn new(hwnd: Option<*mut c_void>) -> Result<Self> {
        #[cfg(target_os = "windows")]
        if hwnd.is_none() {
            return Err(anyhow!("media keys need the main window HWND on Windows"));
        }
        let hwnd_addr = hwnd.map(|p| p as usize);
        let (tx, rx) = flume::unbounded::<Msg>();
        let (ev_tx, events) = flume::unbounded();
        let (init_tx, init_rx) = flume::bounded::<Result<(), String>>(1);
        std::thread::Builder::new().name("glyph-media-keys".into()).spawn(move || {
            let cfg = PlatformConfig {
                dbus_name: "glyph",
                display_name: "Glyph",
                hwnd: hwnd_addr.map(|a| a as *mut c_void),
            };
            let mut controls = match MediaControls::new(cfg) {
                Ok(c) => c,
                Err(e) => {
                    let _ = init_tx.send(Err(format!("{e:?}")));
                    return;
                }
            };
            if let Err(e) = controls.attach(move |e: MediaControlEvent| {
                if let Some(ev) = map_event(&e) {
                    let _ = ev_tx.send(ev);
                }
            }) {
                let _ = init_tx.send(Err(format!("{e:?}")));
                return;
            }
            let _ = init_tx.send(Ok(()));

            let mut last_id: Option<(String, glyph_protocol::ProviderId)> = None;
            while let Ok(msg) = rx.recv() {
                match msg {
                    Msg::Quit => break,
                    Msg::Now { track, playing, position } => {
                        let key = track.as_ref().map(|t| (t.id.clone(), t.source));
                        if key != last_id {
                            let md = match &track {
                                Some(t) => MediaMetadata {
                                    title: Some(&t.title),
                                    artist: Some(&t.artist),
                                    album: t.album.as_deref(),
                                    cover_url: t.cover_url.as_deref(),
                                    duration: t.duration_ms.map(Duration::from_millis),
                                },
                                None => MediaMetadata::default(),
                            };
                            if let Err(e) = controls.set_metadata(md) {
                                tracing::debug!("set_metadata: {e:?}");
                            }
                            last_id = key;
                        }
                        let progress = Some(MediaPosition(position));
                        let pb = match (&track, playing) {
                            (None, _) => MediaPlayback::Stopped,
                            (Some(_), true) => MediaPlayback::Playing { progress },
                            (Some(_), false) => MediaPlayback::Paused { progress },
                        };
                        if let Err(e) = controls.set_playback(pb) {
                            tracing::debug!("set_playback: {e:?}");
                        }
                    }
                }
            }
            let _ = controls.detach();
        })?;
        match init_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self { tx, events }),
            Ok(Err(e)) => Err(anyhow!("media controls unavailable: {e}")),
            Err(_) => Err(anyhow!("media controls thread did not start")),
        }
    }

    pub fn events(&self) -> flume::Receiver<MediaKeyEvent> {
        self.events.clone()
    }

    pub fn set_now_playing(&self, track: Option<&Track>, playing: bool, position: Duration) {
        let _ = self.tx.send(Msg::Now { track: track.cloned(), playing, position });
    }
}

impl Drop for MediaKeys {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Quit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_mapping() {
        assert_eq!(map_event(&MediaControlEvent::Toggle), Some(MediaKeyEvent::Toggle));
        assert_eq!(map_event(&MediaControlEvent::Next), Some(MediaKeyEvent::Next));
        assert_eq!(
            map_event(&MediaControlEvent::SeekBy(SeekDirection::Backward, Duration::from_secs(5))),
            Some(MediaKeyEvent::SeekBackward)
        );
        assert_eq!(map_event(&MediaControlEvent::Raise), None);
    }
}
