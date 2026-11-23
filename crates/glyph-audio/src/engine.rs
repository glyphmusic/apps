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
/// The output must take samples of the playing song: none for this long
/// after play (it never started) or while playing (the stream died without
/// an error, as Android's sometimes does) and the output is opened again.
const STALL_START: Duration = Duration::from_secs(5);
const STALL_PLAYING: Duration = Duration::from_secs(3);
/// Reopens in a row without any sound in between before giving up.
const STALL_RETRIES: u32 = 3;
/// A song whose stream broke midway goes on from there this often.
const RECOVER_TRIES: u32 = 2;
/// ... unless less than this is left of it.
const RECOVER_MIN_LEFT: Duration = Duration::from_secs(8);

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

/// Watches whether the output takes samples of the current song.
/// `watch` is (song id, beat, since when that beat stands). Returns the new
/// watch, whether the output moved on since the last look and whether it
/// stands still for too long.
pub fn watch_output(
    watch: Option<(u64, u64, Instant)>,
    id: u64,
    beat: u64,
    started: bool,
    now: Instant,
) -> (Option<(u64, u64, Instant)>, bool, bool) {
    match watch {
        Some((wid, b, since)) if wid == id && b == beat => {
            let limit = if started { STALL_PLAYING } else { STALL_START };
            (watch, false, now.saturating_duration_since(since) >= limit)
        }
        Some((wid, _, _)) if wid == id => (Some((id, beat, now)), true, false),
        _ => (Some((id, beat, now)), false, false),
    }
}

/// A stream link that expired or is refused: the app has to fetch a new one.
fn link_refused(message: &str) -> bool {
    ["HTTP 403", "HTTP 401", "403 Forbidden", "401 Unauthorized"].iter().any(|s| message.contains(s))
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
    /// The stream failed midway: (message, already reported to the app).
    /// What was buffered plays on; then the song goes on from there.
    broken: Option<(String, bool)>,
}

struct Pending {
    id: u64,
    track: Track,
    is_preview: bool,
    /// Where it plays from: its loudness is measured from there too.
    url: String,
    mime: Option<String>,
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
    /// Output watchdog (see `watch_output`), its reopens in a row and the
    /// last tick (a long gap means this thread was asleep, not the output).
    watch: Option<(u64, u64, Instant)>,
    stall_tries: u32,
    last_tick_at: Instant,
    /// Resumes of the current song after its stream broke.
    recover_tries: u32,
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
            embed_gain: 1.0,
            pause_gain: 1.0,
            pause_fade: None,
            ab_loop: None,
            loop_gain: 1.0,
            loop_jump: None,
            watch: None,
            stall_tries: 0,
            last_tick_at: Instant::now(),
            recover_tries: 0,
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
            Cmd::Load(track, source) => self.load_cmd(track, source),
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

    /// A song from the app. Only here: the player's own reloads (output
    /// reopened, a broken stream resumed, a seek after the end) must never
    /// be taken for the app's answer to `Ended`, or they are dropped and
    /// nothing plays while the time runs on.
    fn load_cmd(&mut self, track: Track, source: PlaybackSource) {
        if let Some((at, t)) = &self.promoted {
            if at.elapsed() < PROMOTE_GRACE && same_track(t, &track) {
                self.promoted = None;
                return;
            }
        }
        self.recover_tries = 0;
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
        self.load(track, source);
    }

    /// Starts `track` afresh (stops everything else first).
    fn load(&mut self, track: Track, source: PlaybackSource) {
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
        self.pending_cur = Some(Pending { id, track, is_preview, url: url.clone(), mime: mime.clone() });
        self.want_loudness();
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
        let had_next = self.next.is_some() || self.pending_next.is_some();
        self.stop_native();
        self.device = None;
        self.opened_device = None;
        if let Some((t, s)) = resume {
            self.resume_at(t, s, pos, had_next);
        }
    }

    /// Opens `track` again and goes on at `pos` (playing or paused as
    /// before). A preloaded next song is gone then: the app is told, so it
    /// hands it over again.
    fn resume_at(&mut self, track: Track, source: PlaybackSource, pos: Duration, had_next: bool) {
        let had_next = had_next || self.next.is_some() || self.pending_next.is_some();
        let was_playing = self.want_play;
        self.load(track, source);
        self.pending_seek = Some(pos);
        if !was_playing {
            self.want_play = false;
        }
        // The time stays where it is while the song opens again.
        self.shared.state.lock().unwrap().position = pos;
        if had_next {
            self.emit(PlayerEvent::PreloadFailed);
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
                crate::resample::set_device_rate(d.config().sample_rate().get());
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
            // Measured once the decoder runs (not competing for the first bytes).
            crate::loudness::analyze(loud_key(&p.track), p.url.clone(), p.mime.clone());
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
            crate::loudness::analyze(loud_key(&p.track), p.url.clone(), p.mime.clone());
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

    /// Only the current and the next song are worth measuring.
    fn want_loudness(&self) {
        let keys = [self.pending_cur.as_ref().map(|p| &p.track), self.cur.as_ref().map(|c| &c.track),
            self.pending_next.as_ref().map(|p| &p.track), self.next.as_ref().map(|n| &n.track)]
            .into_iter()
            .flatten()
            .map(loud_key)
            .collect();
        crate::loudness::set_wanted(keys);
    }

    fn make_slot(&self, id: u64, track: Track, is_preview: bool, o: &Opened) -> Slot {
        let duration = o
            .duration
            .or_else(|| track.duration_ms.map(Duration::from_millis));
        let shared = Arc::new(TrackShared::default());
        if let Some(a) = crate::loudness::lookup(&loud_key(&track)) {
            shared.loudness.set_known(a);
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
            broken: None,
        }
    }

    fn on_decode_error(&mut self, id: u64, message: String) {
        if self.pending_cur.as_ref().is_some_and(|p| p.id == id) {
            self.pending_cur = None;
            self.fail(message);
        } else if self.cur.as_ref().is_some_and(|c| c.id == id) {
            // Mid-stream failure (network gone, link expired): what was
            // buffered plays on, then the song goes on from there
            // (`on_current_finished`) instead of ending early. An expired
            // link is reported at once, so the app fetches a new one meanwhile.
            let refused = link_refused(&message);
            tracing::warn!(track_id = id, refused, "stream failed midway: {message}");
            if refused {
                self.emit(PlayerEvent::Error(message.clone()));
            }
            if let Some(c) = self.cur.as_mut() {
                c.broken = Some((message, refused));
            }
        } else if self.pending_next.as_ref().is_some_and(|p| p.id == id) {
            tracing::warn!("preload failed: {message}");
            self.drop_next();
            self.emit(PlayerEvent::PreloadFailed);
        } else if self.next.as_ref().is_some_and(|n| n.id == id) {
            tracing::warn!("preloaded track failed: {message}");
            self.drop_next();
            self.emit(PlayerEvent::PreloadFailed);
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
        self.pending_next = Some(Pending { id, track, is_preview, url: url.clone(), mime: mime.clone() });
        self.want_loudness();
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
        self.recover_tries = 0;
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
                if self.cur.is_none() && self.pending_cur.is_some() {
                    self.set_status(Status::Loading);
                }
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
        if !was_playing && self.pause_fade.is_some_and(|(_, _, fade_in)| !fade_in) {
            self.set_status(Status::Paused);
            return;
        }
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
                if self.cur.is_some() || self.pending_cur.is_some() {
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
        // Spawn the lazy child WebView2 (if registered) and wait for it to settle.
        self.shared.host.ensure_attached();
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

    /// While a song should play, the output must take its samples. If it
    /// does not (a driver that accepts the stream and never asks for audio,
    /// an Android stream that died without an error) the output is opened
    /// again and the song goes on where it was. `true` if it acted.
    fn check_output(&mut self) -> bool {
        let now = Instant::now();
        // A long gap between ticks means this thread was asleep (phone
        // suspended), not that the output hangs: look afresh.
        let asleep = now.saturating_duration_since(self.last_tick_at) >= Duration::from_secs(1);
        self.last_tick_at = now;
        let Some(c) = self.cur.as_ref().filter(|_| self.want_play && !asleep) else {
            self.watch = None;
            return false;
        };
        let started = c.shared.started.load(Ordering::Relaxed);
        let (watch, moved, stalled) =
            watch_output(self.watch, c.id, c.shared.beat.load(Ordering::Relaxed), started, now);
        self.watch = watch;
        if moved {
            self.stall_tries = 0;
        }
        if !stalled {
            return false;
        }
        self.watch = None;
        self.stall_tries += 1;
        if self.stall_tries > STALL_RETRIES {
            self.stall_tries = 0;
            self.stop_native();
            self.fail("The selected audio output is not responding. Choose another output in Settings > Playback.");
            return true;
        }
        if !started && !self.use_default_buffer {
            // A driver can accept a fixed buffer and then never request
            // samples: retry the same device with its own settings.
            tracing::warn!(device = ?self.opened_device, "audio output did not start; retrying device defaults");
            self.use_default_buffer = true;
        } else {
            tracing::warn!(device = ?self.opened_device, started, tries = self.stall_tries, "audio output stopped taking samples; reopening it");
        }
        self.reopen_device();
        true
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
        if self.check_output() {
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
                } else if !started || cur.shared.buffering.load(Ordering::Relaxed) {
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
        // A song's loudness measured meanwhile: it moves to its gain once.
        for slot in [self.cur.as_ref(), self.next.as_ref()].into_iter().flatten() {
            if !slot.shared.loudness.has_known() {
                if let Some(a) = crate::loudness::lookup(&loud_key(&slot.track)) {
                    slot.shared.loudness.set_known(a);
                }
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
        // The stream broke midway and the buffered part has played: go on
        // from there instead of skipping the rest of the song.
        if let Some(cur) = &self.cur {
            if let Some((message, reported)) = cur.broken.clone() {
                let pos = Duration::from_secs_f64(cur.shared.pos_frames.load(Ordering::Relaxed) as f64 / cur.rate.max(1) as f64);
                let left = cur.duration.map_or(Duration::MAX, |d| d.saturating_sub(pos));
                let resume = self.last_load.clone().filter(|(t, _)| same_track(t, &cur.track));
                match resume {
                    Some((t, s)) if self.recover_tries < RECOVER_TRIES && left > RECOVER_MIN_LEFT => {
                        self.recover_tries += 1;
                        tracing::warn!(at = ?pos, tries = self.recover_tries, "stream broke midway, going on from there");
                        self.resume_at(t, s, pos, false);
                        return;
                    }
                    _ if !reported => self.emit(PlayerEvent::Error(message)),
                    _ => {}
                }
            }
        }
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
            self.pending_next = None;
            self.next_load = None;
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

    #[test]
    fn preload_decode_error_drops_next_and_notifies() {
        let (events_tx, events_rx) = flume::unbounded();
        let shared = Arc::new(Shared {
            host: Arc::new(crate::embed::host::HostQueue::default()),
            state: std::sync::Mutex::new(st()),
            spectrum: std::sync::Mutex::new([0.0; crate::SPECTRUM_BANDS]),
            events_tx,
            events_rx: events_rx.clone(),
        });
        let (cmd_tx, cmd_rx) = flume::unbounded();
        let mut engine = Engine::new(shared, cmd_rx, cmd_tx);
        let mk_track = |id: &str| Track {
            id: id.into(),
            source: glyph_protocol::ProviderId::Deezer,
            title: id.into(),
            artist: "Artist".into(),
            album: None,
            duration_ms: Some(120_000),
            cover_url: None,
            isrc: None,
            permalink: None,
            external_ids: Default::default(),
            explicit: false,
        };
        engine.cur = Some(Slot {
            id: 1,
            track: mk_track("cur"),
            is_preview: false,
            shared: Arc::new(TrackShared::default()),
            ctl: Arc::new(native::DecodeCtl::default()),
            rate: 48_000,
            duration: Some(Duration::from_secs(120)),
            near_end_sent: true,
            broken: None,
        });
        engine.pending_next = Some(Pending { id: 42, track: mk_track("next1"), is_preview: false, url: String::new(), mime: None });
        engine.on_decode_error(42, "network reset".into());
        assert!(engine.pending_next.is_none());
        assert!(!engine.cur.as_ref().unwrap().near_end_sent);
        assert_eq!(events_rx.try_recv().ok(), Some(PlayerEvent::PreloadFailed));

        // Already-decoded `self.next` that hits an HTTP range error later is also dropped.
        let ctl = Arc::new(native::DecodeCtl::default());
        engine.cur.as_mut().unwrap().near_end_sent = true;
        engine.next = Some(Slot {
            id: 43,
            track: mk_track("next2"),
            is_preview: false,
            shared: Arc::new(TrackShared::default()),
            ctl: ctl.clone(),
            rate: 48_000,
            duration: Some(Duration::from_secs(120)),
            near_end_sent: false,
            broken: None,
        });
        engine.on_decode_error(43, "403 Forbidden".into());
        assert!(engine.next.is_none());
        assert!(ctl.stop.load(Ordering::SeqCst));
        assert!(!engine.cur.as_ref().unwrap().near_end_sent);
        assert_eq!(events_rx.try_recv().ok(), Some(PlayerEvent::PreloadFailed));

        // The current song's stream breaks midway: a network error is kept
        // quiet (the song goes on from there later), an expired link is
        // reported at once so the app fetches a new one.
        engine.on_decode_error(1, "stream error: connection reset".into());
        assert!(engine.cur.as_ref().unwrap().broken.as_ref().is_some_and(|(_, reported)| !reported));
        assert!(events_rx.try_recv().is_err());
        engine.on_decode_error(1, "stream error: HTTP 403".into());
        assert!(matches!(events_rx.try_recv(), Ok(PlayerEvent::Error(_))));

        // Nothing left to resume from (no source known): it ends as before.
        engine.cur.as_mut().unwrap().broken = Some(("stream error: reset".into(), false));
        engine.on_current_finished();
        assert_eq!(events_rx.try_recv().ok(), Some(PlayerEvent::Error("stream error: reset".into())));
        assert_eq!(events_rx.try_recv().ok(), Some(PlayerEvent::StatusChanged(Status::Ended)));
        assert_eq!(events_rx.try_recv().ok(), Some(PlayerEvent::Ended));
    }

    #[test]
    fn output_watch_needs_samples_while_playing() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        // First look: nothing to compare with yet.
        let (w, moved, stalled) = watch_output(None, 7, 0, false, at(0));
        assert!(!moved && !stalled);
        // Not started: five seconds to start.
        assert!(!watch_output(w, 7, 0, false, at(4_900)).2);
        assert!(watch_output(w, 7, 0, false, at(5_000)).2);
        // Samples flow: it moved and the clock starts again.
        let (w, moved, stalled) = watch_output(w, 7, 3, true, at(5_000));
        assert!(moved && !stalled);
        // Playing and then no more samples: three seconds.
        assert!(!watch_output(w, 7, 3, true, at(7_900)).2);
        assert!(watch_output(w, 7, 3, true, at(8_000)).2);
        // A new song (crossfade, next song) starts its own clock, however
        // long ago it was preloaded.
        let (w, moved, stalled) = watch_output(w, 8, 0, false, at(9_000));
        assert!(!moved && !stalled);
        assert!(!watch_output(w, 8, 0, false, at(9_016)).2);
    }
}
