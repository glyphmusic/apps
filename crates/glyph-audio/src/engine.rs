//! The player thread: owns the rodio device, the current/next native tracks,
//! the embed backend and the shared state. Everything the UI calls is a
//! message to this thread (see [`crate::Cmd`]).

use crate::embed::{EmbedBackend, EmbedEvent, EmbedPlayState, EventSink};
use crate::native::{self, Opened, Tap, TrackShared, TrackSource};
use crate::spectrum::Analyzer;
use crate::{BackendKind, Cmd, MiniPlayerOptions, PlayerEvent, PlayerState, Shared, Status};
use flume::{Receiver, Sender};
use glyph_protocol::{PlaybackSource, ProviderId, Track};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_millis(16);
/// Pause / resume fade of native playback (no hard cut).
const PAUSE_FADE: Duration = Duration::from_millis(220);
const NEAR_END: Duration = Duration::from_secs(10);
/// A-B loop jump: the fade is the crossfade length, but at most this long
/// (and a quarter of the loop); without crossfade just a click-free dip.
const LOOP_FADE_MAX: Duration = Duration::from_secs(2);
const LOOP_DECLICK: Duration = Duration::from_millis(40);
/// A `load()` of the track that just started gaplessly is the UI reacting to
/// our `Ended`; ignore it for this long.
const PROMOTE_GRACE: Duration = Duration::from_millis(3000);

// ------------------------------------------------------------------ pure parts

/// `NearEnd` is due once, when at most [`NEAR_END`] remain (plus the
/// crossfade, so the next song is ready before the fade starts).
pub fn near_end_due(position: Duration, duration: Option<Duration>, already_sent: bool, crossfade: Duration) -> bool {
    match duration {
        Some(d) if !already_sent && !d.is_zero() => d.saturating_sub(position) <= NEAR_END + crossfade,
        _ => false,
    }
}

/// Equal-power gains (old, new) at fade progress `k` (0..1).
pub fn fade_gains(k: f32) -> (f32, f32) {
    let k = k.clamp(0.0, 1.0);
    let a = k * std::f32::consts::FRAC_PI_2;
    (a.cos(), a.sin())
}

fn loud_key(t: &Track) -> String {
    format!("{:?}:{}", t.source, t.id)
}

/// Volume factor for embedded players around song changes: they cannot
/// overlap (one official player, audio never pulled), so they fade out over
/// the last `cf` and fade in over the first half of it.
pub fn embed_fade(pos: Duration, duration: Option<Duration>, cf: Duration) -> f32 {
    if cf.is_zero() {
        return 1.0;
    }
    let cf_s = cf.as_secs_f32();
    let fade_in = (pos.as_secs_f32() / (cf_s * 0.5).max(0.5)).clamp(0.0, 1.0);
    let fade_out = duration
        .map(|d| (d.saturating_sub(pos).as_secs_f32() / cf_s).clamp(0.0, 1.0))
        .unwrap_or(1.0);
    let k = fade_in.min(fade_out);
    (k * std::f32::consts::FRAC_PI_2).sin()
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
    queued_at: Instant,
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
    /// Chosen output (None = system default) and the name actually opened.
    out_device: Option<String>,
    opened_device: Option<String>,
    device_checked: Instant,
    use_default_buffer: bool,
    /// Set by the output's error callback when the stream died (Bluetooth
    /// or headphones gone, Android rerouting): the next tick reopens it.
    device_lost: Arc<AtomicBool>,
    player: Option<rodio::Player>,
    cur: Option<Slot>,
    next: Option<Slot>,
    pending_cur: Option<Pending>,
    pending_next: Option<Pending>,
    next_id: u64,
    volume: f32,
    want_play: bool,
    last_load: Option<(Track, PlaybackSource)>,
    next_load: Option<(Track, PlaybackSource)>,
    variant_changed: Instant,
    pending_seek: Option<Duration>,
    analyzer: Analyzer,
    tap: Arc<Tap>,
    embed: Option<EmbedBackend>,
    embed_session: u64,
    mini_opts: MiniPlayerOptions,
    promoted: Option<(Instant, Track)>,
    /// Crossfade length (0 = gapless).
    crossfade: Duration,
    /// With crossfade the next track waits on its own player.
    next_player: Option<rodio::Player>,
    /// The previous track while it fades out: (player, slot, fade start).
    fading: Option<(rodio::Player, Slot, Instant)>,
    /// Measured loudness per song this session (LUFS), so replays start
    /// at the right gain.
    loud_cache: std::collections::HashMap<String, f32>,
    /// Embed volume factor of the current fade (1 = none).
    embed_gain: f32,
    /// Native volume factor of the pause / resume fade (1 = none).
    pause_gain: f32,
    /// Running pause fade: (start, gain at start, fading in).
    pause_fade: Option<(Instant, f32, bool)>,
    /// A-B loop of the current song, its volume factor (fading out before
    /// the end point and in after the jump) and when it last jumped.
    ab_loop: Option<(Duration, Duration)>,
    loop_gain: f32,
    loop_jump: Option<Instant>,
}

impl Engine {
    pub(crate) fn new(shared: Arc<Shared>, rx: Receiver<Cmd>, tx: Sender<Cmd>) -> Self {
        let volume = shared.state.lock().unwrap().volume;
        Self {
            shared,
            rx,
            tx,
            device: None,
            out_device: None,
            opened_device: None,
            device_checked: Instant::now(),
            use_default_buffer: false,
            device_lost: Arc::new(AtomicBool::new(false)),
            player: None,
            cur: None,
            next: None,
            pending_cur: None,
            pending_next: None,
            next_id: 1,
            volume,
            want_play: false,
            last_load: None,
            next_load: None,
            variant_changed: Instant::now(),
            pending_seek: None,
            analyzer: Analyzer::new(),
            tap: Arc::new(Tap::default()),
            embed: None,
            embed_session: 0,
            mini_opts: MiniPlayerOptions { always_on_top: false, position: None },
            promoted: None,
            crossfade: Duration::ZERO,
            next_player: None,
            fading: None,
            loud_cache: std::collections::HashMap::new(),
            embed_gain: 1.0,
            pause_gain: 1.0,
            pause_fade: None,
            ab_loop: None,
            loop_gain: 1.0,
            loop_jump: None,
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
            Cmd::DropNext => self.drop_next(),
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
            Cmd::StreamQuality(q) => self.stream_quality(Some(&q)),
            Cmd::Volume(v) => self.set_volume(v),
            Cmd::Speed(s) => self.set_speed(s),
            Cmd::Crossfade(d) => self.crossfade = d,
            Cmd::Loop(l) => self.ab_loop = l,
            Cmd::OutputDevice(name) => {
                if self.out_device != name {
                    self.out_device = name;
                    self.use_default_buffer = false;
                    self.reopen_device();
                }
            }
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
            if self.next.as_ref().is_some_and(|n| same_track(&n.track, &track)) && self.player.is_some() && self.next_player.is_none() {
                if let Some(cur) = self.cur.take() {
                    cur.ctl.stop.store(true, Ordering::SeqCst);
                }
                self.pause_fade = None;
                self.pause_gain = 1.0;
                self.apply_native_volume();
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
        self.pause_fade = None;
        self.pause_gain = 1.0;
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

    fn find_device(name: &str) -> Option<rodio::cpal::Device> {
        use rodio::cpal::traits::{DeviceTrait, HostTrait};
        rodio::cpal::default_host()
            .output_devices()
            .ok()?
            .find(|d| d.description().ok().is_some_and(|x| x.name() == name))
    }

    fn default_device_name() -> Option<String> {
        use rodio::cpal::traits::{DeviceTrait, HostTrait};
        rodio::cpal::default_host().default_output_device()?.description().ok().map(|x| x.name().to_string())
    }

    /// Moves playback to the chosen output: the current song restarts there
    /// at the same position.
    fn reopen_device(&mut self) {
        if self.device.is_none() {
            return;
        }
        let resume = self.last_load.clone().filter(|_| self.backend() == Some(BackendKind::Native));
        let pos = self.shared.state.lock().unwrap().position;
        let was_playing = self.want_play;
        self.stop_native();
        self.device = None;
        self.opened_device = None;
        if let Some((t, s)) = resume {
            self.load(t, s);
            self.pending_seek = Some(pos);
            if !was_playing {
                self.want_play = false;
            }
        }
    }

    /// Following the system default: switch when the default output changes
    /// (headphones plugged in, another device chosen in Windows).
    fn check_default_device(&mut self) {
        if self.out_device.is_some() || self.device.is_none() || self.device_checked.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.device_checked = Instant::now();
        let now = Self::default_device_name();
        if now.is_some() && now != self.opened_device {
            self.reopen_device();
        }
    }

    fn ensure_device(&mut self) -> bool {
        if self.device.is_some() {
            return true;
        }
        // A stream that dies (device unplugged, Bluetooth gone, Android
        // rerouting the output) stops calling for samples without any other
        // sign: playback would hang silently. The tick reopens it instead.
        self.device_lost.store(false, Ordering::Relaxed);
        let lost = self.device_lost.clone();
        let on_error = move |e: rodio::cpal::Error| {
            use rodio::cpal::ErrorKind;
            if matches!(e.kind(), ErrorKind::DeviceNotAvailable | ErrorKind::StreamInvalidated | ErrorKind::HostUnavailable) {
                tracing::warn!("audio output lost: {e}");
                lost.store(true, Ordering::Relaxed);
            } else if !matches!(e.kind(), ErrorKind::Xrun | ErrorKind::RealtimeDenied | ErrorKind::DeviceChanged) {
                tracing::debug!("audio output: {e}");
            }
        };
        // A fixed ~45 ms output buffer first: the OS default (about 10 ms on
        // Windows) runs dry, and crackles, whenever the machine is busy for a
        // moment. Falls back to the device defaults if it is refused.
        let builder = || match self.out_device.as_deref().and_then(Self::find_device) {
            Some(dev) => rodio::DeviceSinkBuilder::from_device(dev),
            None => rodio::DeviceSinkBuilder::from_default_device(),
        };
        let opened = builder()
            .map(|b| b.with_error_callback(on_error.clone()))
            .and_then(|b| if self.use_default_buffer { b.open_sink_or_fallback() }
                else { b.with_buffer_size(rodio::cpal::BufferSize::Fixed(2048)).open_stream() })
            .or_else(|_| builder().and_then(|b| b.with_error_callback(on_error.clone()).open_sink_or_fallback()))
            .or_else(|_| match self.out_device.as_deref().and_then(Self::find_device) {
                Some(dev) => rodio::DeviceSinkBuilder::from_device(dev).and_then(|b| b.open_sink_or_fallback()),
                None => rodio::DeviceSinkBuilder::open_default_sink(),
            });
        self.opened_device = match &self.out_device {
            Some(n) if Self::find_device(n).is_some() => Some(n.clone()),
            _ => Self::default_device_name(),
        };
        match opened {
            Ok(mut d) => {
                let channels = d.config().channel_count().get();
                tracing::info!(device = ?self.opened_device, channels, default_buffer = self.use_default_buffer, "audio output opened");
                // Surround outputs can take multichannel songs as they are.
                crate::spatial::set_device_channels(channels);
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
        p.set_volume(self.out_volume());
        self.player = Some(p);
        Some(())
    }

    /// Stop everything native (decoders + output).
    fn stop_native(&mut self) {
        self.finish_fade();
        if let Some(p) = self.next_player.take() {
            p.stop();
        }
        for s in [self.cur.take(), self.next.take()].into_iter().flatten() {
            s.ctl.stop.store(true, Ordering::SeqCst);
        }
        self.pending_cur = None;
        self.pending_next = None;
        self.next_load = None;
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
        tracing::debug!(track_id = id, "decoder ready for audio output");
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
            tracing::debug!(track_id = id, "track queued on audio output");
        } else if self.pending_next.as_ref().is_some_and(|p| p.id == id) {
            let p = self.pending_next.take().unwrap();
            if self.cur.is_none() || self.player.is_none() {
                opened.ctl.stop.store(true, Ordering::SeqCst);
                return;
            }
            let slot = self.make_slot(id, p.track, p.is_preview, &opened);
            let src = TrackSource::new(opened, slot.shared.clone(), self.tap.clone());
            if self.crossfade.is_zero() {
                self.player.as_ref().unwrap().append(src);
            } else if let Some(dev) = self.device.as_ref() {
                // Waits paused on its own player until the fade starts.
                let p = rodio::Player::connect_new(dev.mixer());
                p.set_volume(0.0);
                p.pause();
                p.append(src);
                self.next_player = Some(p);
            }
            self.next = Some(slot);
        } else {
            opened.ctl.stop.store(true, Ordering::SeqCst); // cancelled meanwhile
        }
    }

    fn make_slot(&self, id: u64, track: Track, is_preview: bool, o: &Opened) -> Slot {
        let duration = o
            .duration
            .or_else(|| track.duration_ms.map(Duration::from_millis));
        let shared = Arc::new(TrackShared::default());
        if let Some(l) = self.loud_cache.get(&loud_key(&track)) {
            shared.loudness.set_known(*l);
        }
        Slot {
            id,
            track,
            is_preview,
            shared,
            ctl: o.ctl.clone(),
            rate: o.rate.get(),
            duration,
            near_end_sent: false,
            queued_at: Instant::now(),
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
        let next_source = source.clone();
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
        self.next_load = Some((track.clone(), next_source));
        self.pending_next = Some(Pending { id, track, is_preview });
        native::spawn_decoder(id, url, mime, self.tx.clone());
    }

    /// The queue changed after the next song was preloaded: drop it, so the
    /// current song ends normally and the UI starts the real next one.
    fn drop_next(&mut self) {
        self.pending_next = None;
        self.next_load = None;
        if let Some(n) = self.next.take() {
            // A gapless next sits behind the current one on the same player;
            // stopped, it ends at once when reached.
            n.ctl.stop.store(true, Ordering::SeqCst);
        }
        if let Some(p) = self.next_player.take() {
            p.stop();
        }
        if let Some(cur) = self.cur.as_mut() {
            cur.near_end_sent = false;
        }
    }

    fn promote_next(&mut self) {
        let Some(n) = self.next.take() else { return };
        if let Some(load) = self.next_load.take() { self.last_load = Some(load); }
        // The loop belongs to the song that just ended.
        if !self.cur.as_ref().is_some_and(|c| same_track(&c.track, &n.track)) {
            self.ab_loop = None;
        }
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
                if self.pause_gain < 1.0 || self.pause_fade.is_some() {
                    self.pause_fade = Some((Instant::now(), self.pause_gain, true));
                    self.apply_native_volume();
                }
                if let Some(p) = &self.player {
                    p.play();
                }
                if let Some((p, _, _)) = &self.fading {
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
        let was_playing = self.want_play;
        self.want_play = false;
        let audible = was_playing
            && self.backend() == Some(BackendKind::Native)
            && self.cur.as_ref().is_some_and(|c| c.shared.started.load(Ordering::Relaxed));
        if audible {
            // Fade out first; `step_pause_fade` pauses the output when silent.
            self.pause_fade = Some((Instant::now(), self.pause_gain, false));
            self.set_status(Status::Paused);
            return;
        }
        self.pause_fade = None;
        self.pause_gain = 0.0;
        self.apply_native_volume();
        if let Some((p, _, _)) = &self.fading {
            p.pause();
        }
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
                // Seeking ends a running fade right away.
                self.finish_fade();
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

    /// Volume for the native players: user volume times the pause fade.
    fn out_volume(&self) -> f32 {
        self.volume * self.pause_gain * self.loop_gain
    }

    fn apply_native_volume(&self) {
        if self.fading.is_none() {
            if let Some(p) = &self.player {
                p.set_volume(self.out_volume());
            }
        }
    }

    /// Advances the pause / resume fade; pauses the output at the end of a
    /// fade-out.
    fn step_pause_fade(&mut self) {
        let Some((at, from, fade_in)) = self.pause_fade else { return };
        let k = (at.elapsed().as_secs_f32() / PAUSE_FADE.as_secs_f32()).min(1.0);
        let to = if fade_in { 1.0 } else { 0.0 };
        // Smoothstep: no click at either end.
        let e = k * k * (3.0 - 2.0 * k);
        self.pause_gain = from + (to - from) * e;
        self.apply_native_volume();
        if k >= 1.0 {
            self.pause_fade = None;
            if !fade_in && !self.want_play {
                if let Some(p) = &self.player {
                    p.pause();
                }
                if let Some((p, _, _)) = &self.fading {
                    p.pause();
                }
            }
        }
    }

    fn set_speed(&mut self, s: f32) {
        let s = if s.is_finite() { s.clamp(0.9, 1.1) } else { 1.0 };
        if let Some(p) = &self.player {
            p.set_speed(s);
        }
    }

    fn set_volume(&mut self, v: f32) {
        let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 };
        self.volume = v;
        self.shared.state.lock().unwrap().volume = v;
        if let Some(p) = &self.player {
            if self.fading.is_none() {
                p.set_volume(self.out_volume());
            }
        }
        if let Some(e) = self.embed.as_mut() {
            e.set_volume(v * self.embed_gain);
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
        // With crossfade on, the song fades in (see `embed_fade`).
        self.embed_gain = if self.crossfade.is_zero() { 1.0 } else { 0.0 };
        if let Err(msg) = self.embed.as_mut().unwrap().load(provider, &embed_id, self.volume * self.embed_gain, session) {
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

    fn stream_quality(&mut self, mode: Option<&str>) {
        let Some((track, mut source)) = self.last_load.clone() else { return; };
        if mode.is_none() {
            if !self.want_play || self.cur.is_none() || self.pending_cur.is_some() || self.fading.is_some() { return; }
            let st = self.shared.state.lock().unwrap();
            if st.duration.is_some_and(|d| d.saturating_sub(st.position) < Duration::from_secs(12)) { return; }
        }
        let PlaybackSource::Stream { ref mut url, .. } = source else { return; };
        let Some((updated,changed)) = super::adaptive::rewrite(url,mode,glyph_protocol::network::class(),self.variant_changed.elapsed()) else { return; };
        *url = updated;
        if !changed { self.last_load = Some((track,source)); return; }
        let pos = self.shared.state.lock().unwrap().position;
        let playing = self.want_play;
        // Each MP4 has its own byte offsets and decoder state. Reopen and seek
        // by timestamp, never splice bytes from different representations.
        self.promoted = None;
        self.drop_next();
        self.load(track,source);
        self.pending_seek = Some(pos);
        self.want_play = playing;
        self.shared.state.lock().unwrap().position = pos;
        self.variant_changed = Instant::now();
    }

    fn tick(&mut self) {
        // At most once a second: an output that fails again right away must
        // not turn into a reopen loop.
        if self.device.is_some() && self.device_checked.elapsed() >= Duration::from_secs(1) && self.device_lost.swap(false, Ordering::Relaxed) {
            tracing::info!(device = ?self.opened_device, "reopening the audio output");
            self.device_checked = Instant::now();
            self.reopen_device();
        }
        self.check_default_device();
        // A driver can accept a fixed buffer and then never request samples.
        // Retrying the same device with its negotiated settings avoids an
        // endless spinner without unexpectedly switching to other speakers.
        let stalled = self.want_play && self.cur.as_ref().is_some_and(|c|
            !c.shared.started.load(Ordering::Relaxed) && c.queued_at.elapsed() >= Duration::from_secs(5));
        if stalled {
            if !self.use_default_buffer {
                tracing::warn!(device = ?self.opened_device, "audio output did not start; retrying device defaults");
                self.use_default_buffer = true;
                self.reopen_device();
            } else {
                self.stop_native();
                self.fail("The selected audio output is not responding. Choose another output in Settings > Playback.");
            }
            return;
        }
        self.stream_quality(None);
        let mut analysed = false;
        if let Some(cur) = self.cur.as_mut() {
            let finished = cur.shared.finished.load(Ordering::SeqCst);
            let started = cur.shared.started.load(Ordering::Relaxed)
                && cur.shared.playback_gen.load(Ordering::Acquire) == cur.ctl.gen.load(Ordering::SeqCst);
            let pos = Duration::from_secs_f64(cur.shared.pos_frames.load(Ordering::Relaxed) as f64 / cur.rate.max(1) as f64);
            let duration = cur.duration;
            let due = started && near_end_due(pos, duration, cur.near_end_sent, self.crossfade);
            let fade_due = !self.crossfade.is_zero()
                && self.fading.is_none()
                && self.next_player.is_some()
                && started
                && duration.is_some_and(|d| d.saturating_sub(pos) <= self.crossfade);
            if due {
                cur.near_end_sent = true;
            }
            let rate = cur.rate;
            let underruns = cur.shared.underruns.swap(0, Ordering::Relaxed);
            if underruns > 0 {
                tracing::warn!(track_id = cur.id, underruns, "decoder audio buffer underrun");
            }
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
                    let mut recent = [0.0; crate::spectrum::FFT_SIZE];
                    let count = {
                        let g = self.tap.samples.lock().unwrap();
                        let n = g.len().min(recent.len());
                        recent[..n].copy_from_slice(&g[g.len() - n..]);
                        n
                    };
                    let bands = self.analyzer.process(&recent[..count], rate);
                    *self.shared.spectrum.lock().unwrap() = bands;
                    analysed = true;
                }
                if fade_due {
                    self.start_fade();
                }
            } else {
                self.on_current_finished();
            }
        }
        self.step_fade();
        self.step_pause_fade();
        self.step_loop();
        // Remember what was measured (replays start at the right gain).
        if let Some(cur) = &self.cur {
            if let Some(l) = cur.shared.loudness.measured() {
                self.loud_cache.insert(loud_key(&cur.track), l);
            }
        }
        // Embedded players: volume fades at song changes.
        if self.backend() == Some(BackendKind::Embed) {
            let (pos, dur) = {
                let st = self.shared.state.lock().unwrap();
                (st.position, st.duration)
            };
            let g = embed_fade(pos, dur, self.crossfade);
            if (g - self.embed_gain).abs() > 0.02 || (g >= 1.0 && self.embed_gain < 1.0) {
                self.embed_gain = g;
                let v = self.volume * g;
                if let Some(e) = self.embed.as_mut() {
                    e.set_volume(v);
                }
            }
        } else {
            self.embed_gain = 1.0;
        }
        if !analysed {
            let bands = self.analyzer.decay();
            *self.shared.spectrum.lock().unwrap() = bands;
        }
    }

    // ---- A-B loop

    /// Fades out towards the loop's end point, jumps back to its start and
    /// fades in again.
    fn step_loop(&mut self) {
        let smooth = |k: f32| {
            let k = k.clamp(0.0, 1.0);
            k * k * (3.0 - 2.0 * k)
        };
        let mut g = 1.0;
        if let Some((a, b)) = self.ab_loop {
            let fade = self.crossfade.min(b.saturating_sub(a) / 4).min(LOOP_FADE_MAX).max(LOOP_DECLICK);
            if let Some(t) = self.loop_jump {
                let k = t.elapsed().as_secs_f32() / fade.as_secs_f32();
                if k >= 1.0 && t.elapsed() >= Duration::from_millis(250) {
                    self.loop_jump = None;
                } else {
                    g = smooth(k);
                }
            }
            // Right after a jump the decoder may still report the old
            // position: wait for it before checking the end point again.
            let settling = self.loop_jump.is_some_and(|t| t.elapsed() < Duration::from_millis(250));
            let pos = self.cur.as_ref().filter(|c| c.shared.started.load(Ordering::Relaxed)
                && c.shared.playback_gen.load(Ordering::Acquire) == c.ctl.gen.load(Ordering::SeqCst)
                && !c.shared.finished.load(Ordering::SeqCst)).map(|c| {
                Duration::from_secs_f64(c.shared.pos_frames.load(Ordering::Relaxed) as f64 / c.rate.max(1) as f64)
            });
            if let Some(pos) = pos.filter(|_| !settling && self.fading.is_none()) {
                if pos >= b {
                    self.seek(a);
                    self.loop_jump = Some(Instant::now());
                    g = 0.0;
                } else if pos >= a && b - pos < fade {
                    g = g.min(smooth((b - pos).as_secs_f32() / fade.as_secs_f32()));
                }
            }
        } else {
            self.loop_jump = None;
        }
        if (g - self.loop_gain).abs() > 1e-4 {
            self.loop_gain = g;
            self.apply_native_volume();
        }
    }

    // ---- crossfade

    /// The next song starts on its own player while the current one fades out.
    fn start_fade(&mut self) {
        let (Some(next_player), Some(old_player), Some(old)) = (self.next_player.take(), self.player.take(), self.cur.take()) else {
            return;
        };
        next_player.set_volume(0.0);
        if self.want_play {
            next_player.play();
        }
        self.player = Some(next_player);
        self.fading = Some((old_player, old, Instant::now()));
        // The UI moves on now (its `load()` of this track is ignored as promoted).
        // Promote first so the UI already sees the new track with `Ended`.
        self.promote_next();
        self.emit(PlayerEvent::Ended);
    }

    fn step_fade(&mut self) {
        let Some((old_player, _, at)) = &self.fading else { return };
        let k = at.elapsed().as_secs_f32() / self.crossfade.as_secs_f32().max(0.001);
        let (g_old, g_new) = fade_gains(k);
        old_player.set_volume(self.out_volume() * g_old);
        if let Some(p) = &self.player {
            p.set_volume(self.out_volume() * g_new);
        }
        if k >= 1.0 {
            self.finish_fade();
        }
    }

    /// Drops the faded-out track and puts the new one at full volume.
    fn finish_fade(&mut self) {
        if let Some((old_player, old, _)) = self.fading.take() {
            old.ctl.stop.store(true, Ordering::SeqCst);
            old_player.stop();
            if let Some(p) = &self.player {
                p.set_volume(self.out_volume());
            }
        }
    }

    fn on_current_finished(&mut self) {
        if let Some(cur) = &self.cur {
            if let Some(d) = cur.duration {
                self.shared.state.lock().unwrap().position = d;
            }
        }
        if self.next.is_some() {
            // A seek can skip the crossfade window. The next source then
            // still sits paused on its own player: explicitly start it.
            if let Some(next_player) = self.next_player.take() {
                if let Some(old) = self.player.replace(next_player) { old.stop(); }
                if let Some(player) = &self.player {
                    player.set_volume(self.out_volume());
                    if self.want_play { player.play(); }
                }
            }
            self.promote_next();
            self.emit(PlayerEvent::Ended);
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
        let z = Duration::ZERO;
        assert!(!near_end_due(Duration::from_secs(80), d, false, z));
        assert!(near_end_due(Duration::from_secs(90), d, false, z));
        assert!(!near_end_due(Duration::from_secs(95), d, true, z));
        assert!(!near_end_due(Duration::from_secs(95), None, false, z));
        // With a 12 s crossfade the next song is requested 22 s before the end.
        assert!(near_end_due(Duration::from_secs(78), d, false, Duration::from_secs(12)));
    }

    #[test]
    fn crossfade_gains_are_equal_power() {
        assert_eq!(fade_gains(0.0), (1.0, 0.0));
        let (a, b) = fade_gains(0.5);
        assert!((a * a + b * b - 1.0).abs() < 1e-5);
        let (a, b) = fade_gains(1.0);
        assert!(a.abs() < 1e-6 && (b - 1.0).abs() < 1e-6);
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
