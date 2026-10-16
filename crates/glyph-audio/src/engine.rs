//! The player thread: owns the rodio device, the current/next native tracks,
//! the embed backend and the shared state. Everything the UI calls is a
//! message to this thread (see [`crate::Cmd`]).

use crate::embed::{EmbedBackend, EmbedEvent, EmbedPlayState, EventSink};
use crate::native::{self, Opened, Tap, TrackShared, TrackSource};
use crate::spectrum::Analyzer;
use crate::{BackendKind, Cmd, MiniPlayerOptions, PlayerEvent, PlayerState, Shared, Status};
use flume::{Receiver, Sender};
use glyph_protocol::{PlaybackSource, ProviderId, Track};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_millis(16);
const NEAR_END: Duration = Duration::from_secs(10);
/// A `load()` of the track that just started gaplessly is the UI reacting to
/// our `Ended`; ignore it for this long.
const PROMOTE_GRACE: Duration = Duration::from_millis(3000);

// ------------------------------------------------------------------ pure parts

/// `NearEnd` is due once, when at most [`NEAR_END`] remain.
pub fn near_end_due(position: Duration, duration: Option<Duration>, already_sent: bool) -> bool {
    match duration {
        Some(d) if !already_sent && !d.is_zero() => d.saturating_sub(position) <= NEAR_END,
        _ => false,
    }
}

fn same_track(a: &Track, b: &Track) -> bool {
    a.id == b.id && a.source == b.source
}

/// Apply a page event to the player state. Returns the events to publish.
pub fn reduce_embed(state: &mut PlayerState, ev: EmbedEvent) -> Vec<PlayerEvent> {
    let mut out = Vec::new();
    let set = |state: &mut PlayerState, s: Status, out: &mut Vec<PlayerEvent>| {
        if state.status != s {
            state.status = s;
            out.push(PlayerEvent::StatusChanged(s));
        }
    };
    match ev {
        EmbedEvent::PageReady | EmbedEvent::Ready | EmbedEvent::Closed => {}
        EmbedEvent::State(s) => match s {
            EmbedPlayState::Playing => set(state, Status::Playing, &mut out),
            EmbedPlayState::Paused => set(state, Status::Paused, &mut out),
            EmbedPlayState::Buffering => set(state, Status::Loading, &mut out),
            EmbedPlayState::Cued | EmbedPlayState::Unstarted => {}
            EmbedPlayState::Ended => {
                if let Some(d) = state.duration {
                    state.position = d;
                }
                set(state, Status::Ended, &mut out);
                out.push(PlayerEvent::Ended);
            }
        },
        EmbedEvent::Time { pos, dur } => {
            state.position = pos;
            if dur.is_some() {
                state.duration = dur;
            }
        }
        EmbedEvent::Error(msg) => {
            set(state, Status::Idle, &mut out);
            out.push(PlayerEvent::Error(msg));
        }
    }
    out
}

// ------------------------------------------------------------------ engine

struct Slot {
    id: u64,
    track: Track,
    is_preview: bool,
    shared: Arc<TrackShared>,
    ctl: Arc<native::DecodeCtl>,
    rate: u32,
    duration: Option<Duration>,
    near_end_sent: bool,
}

struct Pending {
    id: u64,
    track: Track,
    is_preview: bool,
}

pub(crate) struct Engine {
    shared: Arc<Shared>,
    rx: Receiver<Cmd>,
    tx: Sender<Cmd>,
    device: Option<rodio::MixerDeviceSink>,
    player: Option<rodio::Player>,
    cur: Option<Slot>,
    next: Option<Slot>,
    pending_cur: Option<Pending>,
    pending_next: Option<Pending>,
    next_id: u64,
    volume: f32,
    want_play: bool,
    last_load: Option<(Track, PlaybackSource)>,
    pending_seek: Option<Duration>,
    analyzer: Analyzer,
    tap: Arc<Tap>,
    embed: Option<EmbedBackend>,
    embed_session: u64,
    mini_opts: MiniPlayerOptions,
    promoted: Option<(Instant, Track)>,
}

impl Engine {
    pub(crate) fn new(shared: Arc<Shared>, rx: Receiver<Cmd>, tx: Sender<Cmd>) -> Self {
        let volume = shared.state.lock().unwrap().volume;
        Self {
            shared,
            rx,
            tx,
            device: None,
            player: None,
            cur: None,
            next: None,
            pending_cur: None,
            pending_next: None,
            next_id: 1,
            volume,
            want_play: false,
            last_load: None,
            pending_seek: None,
            analyzer: Analyzer::new(),
            tap: Arc::new(Tap::default()),
            embed: None,
            embed_session: 0,
            mini_opts: MiniPlayerOptions { always_on_top: false, position: None },
            promoted: None,
        }
    }

    pub(crate) fn run(mut self) {
        let mut last_tick = Instant::now();
        loop {
            match self.rx.recv_timeout(TICK) {
                Ok(Cmd::Shutdown) => break,
                Ok(cmd) => self.handle(cmd),
                Err(flume::RecvTimeoutError::Timeout) => {}
                Err(flume::RecvTimeoutError::Disconnected) => break,
            }
            if last_tick.elapsed() >= TICK {
                last_tick = Instant::now();
                self.tick();
            }
        }
        self.stop_native();
        if let Some(e) = self.embed.as_mut() {
            e.park();
        }
    }

    // ---- helpers

    fn emit(&self, e: PlayerEvent) {
        let _ = self.shared.events_tx.send(e);
    }

    fn set_status(&self, s: Status) {
        let mut st = self.shared.state.lock().unwrap();
        if st.status != s {
            st.status = s;
            drop(st);
            self.emit(PlayerEvent::StatusChanged(s));
        }
    }

    fn fail(&self, msg: impl Into<String>) {
        self.set_status(Status::Idle);
        self.emit(PlayerEvent::Error(msg.into()));
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn set_track_state(&self, track: &Track, backend: BackendKind, is_preview: bool, duration: Option<Duration>) {
        let mut st = self.shared.state.lock().unwrap();
        st.track = Some(track.clone());
        st.provider = Some(track.source);
        st.backend = Some(backend);
        st.is_preview = is_preview;
        st.position = Duration::ZERO;
        st.duration = duration.or_else(|| track.duration_ms.map(Duration::from_millis));
    }

    // ---- command dispatch

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Load(track, source) => self.load(track, source),
            Cmd::Preload(track, source) => self.preload(track, source),
            Cmd::Play => self.play(),
            Cmd::Pause => self.pause(),
            Cmd::Toggle => {
                if self.want_play && self.is_active() {
                    self.pause()
                } else {
                    self.play()
                }
            }
            Cmd::Stop => self.stop(),
            Cmd::Seek(p) => self.seek(p),
            Cmd::Volume(v) => self.set_volume(v),
            Cmd::MiniOptions(o) => {
                self.mini_opts = o;
                if let Some(e) = self.embed.as_mut() {
                    e.set_options(o);
                }
            }
            Cmd::Opened { id, opened } => self.on_opened(id, opened),
            Cmd::DecodeError { id, message } => self.on_decode_error(id, message),
            Cmd::Embed(session, ev) => self.on_embed(session, ev),
            Cmd::HostAttached => self.on_host_attached(),
            Cmd::Shutdown => {}
        }
    }

    fn is_active(&self) -> bool {
        matches!(self.shared.state.lock().unwrap().status, Status::Playing | Status::Loading)
    }

    fn backend(&self) -> Option<BackendKind> {
        self.shared.state.lock().unwrap().backend
    }

    // ---- load / stop

    fn load(&mut self, track: Track, source: PlaybackSource) {
        if let Some((at, t)) = &self.promoted {
            if at.elapsed() < PROMOTE_GRACE && same_track(t, &track) {
                self.promoted = None;
                return;
            }
        }
        // Next track was preloaded and queued behind the current one: switch gaplessly.
        if matches!(source, PlaybackSource::Stream { .. } | PlaybackSource::Preview { .. }) {
            if self.next.as_ref().is_some_and(|n| same_track(&n.track, &track)) && self.player.is_some() {
                if let Some(cur) = self.cur.take() {
                    cur.ctl.stop.store(true, Ordering::SeqCst);
                }
                if let Some(p) = &self.player {
                    p.skip_one();
                    p.play();
                }
                self.want_play = true;
                self.last_load = Some((track, source));
                self.promote_next();
                return;
            }
        }
        self.stop_all();
        self.want_play = true;
        self.pending_seek = None;
        self.last_load = Some((track.clone(), source.clone()));
        match source {
            PlaybackSource::Stream { url, mime, .. } => self.start_native(track, url, mime, false),
            PlaybackSource::Preview { url, .. } => self.start_native(track, url, None, true),
            PlaybackSource::Embed { provider, embed_id } => self.start_embed(track, provider, embed_id),
        }
    }

    fn start_native(&mut self, track: Track, url: String, mime: Option<String>, is_preview: bool) {
        if !self.ensure_device() {
            return;
        }
        let id = self.alloc_id();
        self.set_track_state(&track, BackendKind::Native, is_preview, None);
        self.pending_cur = Some(Pending { id, track, is_preview });
        self.set_status(Status::Loading);
        native::spawn_decoder(id, url, mime, self.tx.clone());
    }

    fn ensure_device(&mut self) -> bool {
        if self.device.is_some() {
            return true;
        }
        match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(mut d) => {
                d.log_on_drop(false);
                self.device = Some(d);
                true
            }
            Err(e) => {
                self.fail(format!("no audio output device: {e}"));
                false
            }
        }
    }

    fn new_player(&mut self) -> Option<()> {
        let dev = self.device.as_ref()?;
        let p = rodio::Player::connect_new(dev.mixer());
        p.set_volume(self.volume);
        self.player = Some(p);
        Some(())
    }

    /// Stop everything native (decoders + output).
    fn stop_native(&mut self) {
        for s in [self.cur.take(), self.next.take()].into_iter().flatten() {
            s.ctl.stop.store(true, Ordering::SeqCst);
        }
        self.pending_cur = None;
        self.pending_next = None;
        if let Some(p) = self.player.take() {
            p.stop();
        }
        if let Ok(mut t) = self.tap.samples.lock() {
            t.clear();
        }
    }

    fn park_embed(&mut self) {
        if let Some(e) = self.embed.as_mut() {
            e.park();
        }
    }

    fn stop_all(&mut self) {
        self.stop_native();
        self.park_embed();
        // Invalidate in-flight embed events.
        self.embed_session += 1;
    }

    fn stop(&mut self) {
        self.stop_all();
        self.want_play = false;
        self.last_load = None;
        self.pending_seek = None;
        {
            let mut st = self.shared.state.lock().unwrap();
            st.track = None;
            st.provider = None;
            st.backend = None;
            st.is_preview = false;
            st.position = Duration::ZERO;
            st.duration = None;
        }
        self.set_status(Status::Idle);
    }

    // ---- native events

    fn on_opened(&mut self, id: u64, opened: Opened) {
        if self.pending_cur.as_ref().is_some_and(|p| p.id == id) {
            let p = self.pending_cur.take().unwrap();
            if self.new_player().is_none() {
                opened.ctl.stop.store(true, Ordering::SeqCst);
                self.fail("audio output not available");
                return;
            }
            let slot = self.make_slot(id, p.track, p.is_preview, &opened);
            let src = TrackSource::new(opened, slot.shared.clone(), self.tap.clone());
            if let Some(seek) = self.pending_seek.take() {
                slot.ctl.request_seek(seek.as_secs_f64());
            }
            let player = self.player.as_ref().unwrap();
            player.append(src);
            if self.want_play {
                player.play();
            } else {
                player.pause();
            }
            {
                let mut st = self.shared.state.lock().unwrap();
                if slot.duration.is_some() {
                    st.duration = slot.duration;
                }
            }
            self.cur = Some(slot);
        } else if self.pending_next.as_ref().is_some_and(|p| p.id == id) {
            let p = self.pending_next.take().unwrap();
            if self.cur.is_none() || self.player.is_none() {
                opened.ctl.stop.store(true, Ordering::SeqCst);
                return;
            }
            let slot = self.make_slot(id, p.track, p.is_preview, &opened);
            let src = TrackSource::new(opened, slot.shared.clone(), self.tap.clone());
            self.player.as_ref().unwrap().append(src);
            self.next = Some(slot);
        } else {
            opened.ctl.stop.store(true, Ordering::SeqCst); // cancelled meanwhile
        }
    }

    fn make_slot(&self, id: u64, track: Track, is_preview: bool, o: &Opened) -> Slot {
        let duration = o
            .duration
            .or_else(|| track.duration_ms.map(Duration::from_millis));
        Slot {
            id,
            track,
            is_preview,
            shared: Arc::new(TrackShared::default()),
            ctl: o.ctl.clone(),
            rate: o.rate.get(),
            duration,
            near_end_sent: false,
        }
    }

    fn on_decode_error(&mut self, id: u64, message: String) {
        if self.pending_cur.as_ref().is_some_and(|p| p.id == id) {
            self.pending_cur = None;
            self.fail(message);
        } else if self.cur.as_ref().is_some_and(|c| c.id == id) {
            // Mid-stream failure: the source drains and ends; the UI gets Error then Ended.
            self.emit(PlayerEvent::Error(message));
        } else if self.pending_next.as_ref().is_some_and(|p| p.id == id) {
            tracing::warn!("preload failed: {message}");
            self.pending_next = None;
        } else if self.next.as_ref().is_some_and(|n| n.id == id) {
            tracing::warn!("preloaded track failed: {message}");
        }
    }

    fn preload(&mut self, track: Track, source: PlaybackSource) {
        let (url, mime, is_preview) = match source {
            PlaybackSource::Stream { url, mime, .. } => (url, mime, false),
            PlaybackSource::Preview { url, .. } => (url, None, true),
            PlaybackSource::Embed { .. } => return,
        };
        if self.cur.is_none() || self.backend() != Some(BackendKind::Native) {
            return;
        }
        if self.next.as_ref().is_some_and(|n| same_track(&n.track, &track))
            || self.pending_next.as_ref().is_some_and(|p| same_track(&p.track, &track))
        {
            return;
        }
        // Replacing an already queued next track would need a player rebuild; only
        // allow replacing one that has not been appended yet.
        if self.next.is_some() {
            return;
        }
        let id = self.alloc_id();
        self.pending_next = Some(Pending { id, track, is_preview });
        native::spawn_decoder(id, url, mime, self.tx.clone());
    }

    fn promote_next(&mut self) {
        let Some(n) = self.next.take() else { return };
        {
            let mut st = self.shared.state.lock().unwrap();
            st.track = Some(n.track.clone());
            st.provider = Some(n.track.source);
            st.is_preview = n.is_preview;
            st.position = Duration::ZERO;
            st.duration = n.duration;
        }
        self.promoted = Some((Instant::now(), n.track.clone()));
        self.cur = Some(n);
    }

    // ---- transport

    fn play(&mut self) {
        match self.backend() {
            Some(BackendKind::Embed) => {
                self.want_play = true;
                if let Some(e) = self.embed.as_mut() {
                    e.play();
                }
            }
            Some(BackendKind::Native) => {
                let status = self.shared.state.lock().unwrap().status;
                if status == Status::Ended || (self.cur.is_none() && self.pending_cur.is_none()) {
                    if let Some((t, s)) = self.last_load.clone() {
                        self.load(t, s);
                    }
                    return;
                }
                self.want_play = true;
                if let Some(p) = &self.player {
                    p.play();
                }
            }
            None => {
                if let Some((t, s)) = self.last_load.clone() {
                    self.load(t, s);
                }
            }
        }
    }

    fn pause(&mut self) {
        self.want_play = false;
        match self.backend() {
            Some(BackendKind::Embed) => {
                if let Some(e) = self.embed.as_mut() {
                    e.pause();
                }
                self.set_status(Status::Paused);
            }
            Some(BackendKind::Native) => {
                if let Some(p) = &self.player {
                    p.pause();
                }
                if self.cur.is_some() {
                    self.set_status(Status::Paused);
                }
            }
            None => {}
        }
    }

    fn seek(&mut self, pos: Duration) {
        let duration = self.shared.state.lock().unwrap().duration;
        let pos = duration.map_or(pos, |d| pos.min(d));
        match self.backend() {
            Some(BackendKind::Embed) => {
                if let Some(e) = self.embed.as_mut() {
                    e.seek(pos);
                }
                self.shared.state.lock().unwrap().position = pos;
            }
            Some(BackendKind::Native) => {
                if self.pending_cur.is_some() {
                    self.pending_seek = Some(pos);
                } else if self.cur.as_ref().is_some_and(|c| !c.ctl.done.load(Ordering::SeqCst))
                    && self.cur.as_ref().is_some_and(|c| !c.shared.finished.load(Ordering::SeqCst))
                {
                    let cur = self.cur.as_mut().unwrap();
                    cur.ctl.request_seek(pos.as_secs_f64());
                    if cur.duration.is_some_and(|d| d.saturating_sub(pos) > NEAR_END + Duration::from_secs(1))
                        && self.next.is_none()
                        && self.pending_next.is_none()
                    {
                        cur.near_end_sent = false;
                    }
                } else if let Some((t, s)) = self.last_load.clone() {
                    // Decoder already finished (or track ended): restart at `pos`.
                    let was_playing = self.want_play;
                    self.load(t, s);
                    self.want_play = was_playing || self.want_play;
                    self.pending_seek = Some(pos);
                } else {
                    return;
                }
                self.shared.state.lock().unwrap().position = pos;
            }
            None => {}
        }
    }

    fn set_volume(&mut self, v: f32) {
        let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 };
        self.volume = v;
        self.shared.state.lock().unwrap().volume = v;
        if let Some(p) = &self.player {
            p.set_volume(v);
        }
        if let Some(e) = self.embed.as_mut() {
            e.set_volume(v);
        }
    }

    // ---- embed

    fn start_embed(&mut self, track: Track, provider: ProviderId, embed_id: String) {
        if crate::embed::provider_key(provider).is_none() {
            self.fail(format!("{} has no embed player", provider.display_name()));
            return;
        }
        // Windows attaches the child webview asynchronously; give it a moment.
        self.shared.host.wait_settled(Duration::from_secs(10));
        // A popup/browser backend must be replaced once the host child webview exists.
        if self.embed.as_ref().is_some_and(|e| !e.is_host()) && self.shared.host.is_attached() {
            if let Some(mut e) = self.embed.take() {
                e.park();
            }
        }
        if self.embed.is_none() {
            let tx = self.tx.clone();
            let sink: EventSink = Arc::new(move |s, ev| {
                let _ = tx.send(Cmd::Embed(s, ev));
            });
            match EmbedBackend::start(sink, self.mini_opts, Some(self.shared.host.clone())) {
                Ok(e) => self.embed = Some(e),
                Err(e) => {
                    self.fail(format!("mini-player unavailable: {e:#}"));
                    return;
                }
            }
        }
        self.embed_session += 1;
        let session = self.embed_session;
        self.set_track_state(&track, BackendKind::Embed, false, None);
        self.set_status(Status::Loading);
        if let Err(msg) = self.embed.as_mut().unwrap().load(provider, &embed_id, self.volume, session) {
            self.fail(msg);
        }
    }

    fn on_host_attached(&mut self) {
        let Some(e) = self.embed.as_mut() else { return };
        if e.is_host() {
            return;
        }
        // Move a running popup/browser embed into the main window.
        e.park();
        self.embed = None;
        if self.backend() == Some(BackendKind::Embed) {
            if let Some((t, s)) = self.last_load.clone() {
                self.load(t, s);
            }
        }
    }

    fn on_embed(&mut self, session: u64, ev: EmbedEvent) {
        match &ev {
            EmbedEvent::PageReady => {
                if let Some(e) = self.embed.as_mut() {
                    e.page_ready();
                }
                return;
            }
            EmbedEvent::Closed => {
                if self.backend() == Some(BackendKind::Embed) {
                    self.want_play = false;
                    if let Some(e) = self.embed.as_mut() {
                        e.pause();
                    }
                    self.set_status(Status::Paused);
                }
                return;
            }
            _ => {}
        }
        if self.backend() != Some(BackendKind::Embed) || session != self.embed_session {
            return;
        }
        let events = {
            let mut st = self.shared.state.lock().unwrap();
            reduce_embed(&mut st, ev)
        };
        for e in events {
            self.emit(e);
        }
    }

    // ---- periodic

    fn tick(&mut self) {
        let mut analysed = false;
        if let Some(cur) = self.cur.as_mut() {
            let finished = cur.shared.finished.load(Ordering::SeqCst);
            let started = cur.shared.started.load(Ordering::Relaxed);
            let pos = Duration::from_secs_f64(cur.shared.pos_frames.load(Ordering::Relaxed) as f64 / cur.rate.max(1) as f64);
            let duration = cur.duration;
            let due = near_end_due(pos, duration, cur.near_end_sent);
            if due {
                cur.near_end_sent = true;
            }
            let rate = cur.rate;
            if !finished {
                {
                    let mut st = self.shared.state.lock().unwrap();
                    if started {
                        st.position = pos;
                    }
                    if st.duration.is_none() {
                        st.duration = duration;
                    }
                }
                let desired = if !self.want_play {
                    Status::Paused
                } else if !started {
                    Status::Loading
                } else {
                    Status::Playing
                };
                self.set_status(desired);
                if due {
                    self.emit(PlayerEvent::NearEnd);
                }
                if self.want_play && started {
                    let recent: Vec<f32> = {
                        let g = self.tap.samples.lock().unwrap();
                        let n = g.len().min(crate::spectrum::FFT_SIZE);
                        g[g.len() - n..].to_vec()
                    };
                    let bands = self.analyzer.process(&recent, rate);
                    *self.shared.spectrum.lock().unwrap() = bands;
                    analysed = true;
                }
            } else {
                self.on_current_finished();
            }
        }
        if !analysed {
            let bands = self.analyzer.decay();
            *self.shared.spectrum.lock().unwrap() = bands;
        }
    }

    fn on_current_finished(&mut self) {
        if let Some(cur) = &self.cur {
            if let Some(d) = cur.duration {
                self.shared.state.lock().unwrap().position = d;
            }
        }
        if self.next.is_some() {
            self.emit(PlayerEvent::Ended);
            self.promote_next();
        } else {
            self.cur = None;
            if let Some(p) = self.player.take() {
                p.stop();
            }
            self.set_status(Status::Ended);
            self.emit(PlayerEvent::Ended);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st() -> PlayerState {
        PlayerState {
            status: Status::Loading,
            track: None,
            provider: None,
            backend: Some(BackendKind::Embed),
            is_preview: false,
            position: Duration::ZERO,
            duration: Some(Duration::from_secs(200)),
            volume: 1.0,
        }
    }

    #[test]
    fn near_end_fires_once_within_ten_seconds() {
        let d = Some(Duration::from_secs(100));
        assert!(!near_end_due(Duration::from_secs(80), d, false));
        assert!(near_end_due(Duration::from_secs(90), d, false));
        assert!(!near_end_due(Duration::from_secs(95), d, true));
        assert!(!near_end_due(Duration::from_secs(95), None, false));
    }

    #[test]
    fn embed_state_transitions() {
        let mut s = st();
        let ev = reduce_embed(&mut s, EmbedEvent::State(EmbedPlayState::Playing));
        assert_eq!(ev, vec![PlayerEvent::StatusChanged(Status::Playing)]);
        // same state again -> no duplicate event
        assert!(reduce_embed(&mut s, EmbedEvent::State(EmbedPlayState::Playing)).is_empty());
        let ev = reduce_embed(&mut s, EmbedEvent::State(EmbedPlayState::Paused));
        assert_eq!(ev, vec![PlayerEvent::StatusChanged(Status::Paused)]);
        let ev = reduce_embed(&mut s, EmbedEvent::State(EmbedPlayState::Buffering));
        assert_eq!(ev, vec![PlayerEvent::StatusChanged(Status::Loading)]);
    }

    #[test]
    fn embed_time_and_end() {
        let mut s = st();
        reduce_embed(&mut s, EmbedEvent::Time { pos: Duration::from_secs(5), dur: Some(Duration::from_secs(180)) });
        assert_eq!(s.position, Duration::from_secs(5));
        assert_eq!(s.duration, Some(Duration::from_secs(180)));
        // unknown duration keeps the old one
        reduce_embed(&mut s, EmbedEvent::Time { pos: Duration::from_secs(6), dur: None });
        assert_eq!(s.duration, Some(Duration::from_secs(180)));
        s.status = Status::Playing;
        let ev = reduce_embed(&mut s, EmbedEvent::State(EmbedPlayState::Ended));
        assert_eq!(ev, vec![PlayerEvent::StatusChanged(Status::Ended), PlayerEvent::Ended]);
        assert_eq!(s.position, Duration::from_secs(180));
    }

    #[test]
    fn embed_error_goes_idle() {
        let mut s = st();
        let ev = reduce_embed(&mut s, EmbedEvent::Error("embedding disabled".into()));
        assert_eq!(
            ev,
            vec![PlayerEvent::StatusChanged(Status::Idle), PlayerEvent::Error("embedding disabled".into())]
        );
    }
}
