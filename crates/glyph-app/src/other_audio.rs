//! Pause when another app plays sound, and play on when it stops.
//!
//! Android only: the app shell watches the other apps (audio focus and the
//! active players, see `PlaybackService.java`) and calls [`report`] whenever
//! what it sees changes. Its tick calls [`tick`], which pauses and resumes.
//!
//! Rules:
//! - Glyph only pauses when the other sound *starts* while Glyph plays.
//!   Pressing play in Glyph while a video runs is not undone.
//! - It plays on when the other sound is really over, but only when that is
//!   sure (Android handed the sound back, or the other player was seen and
//!   then stopped). Never after a music app (Spotify, YouTube Music, ...),
//!   never after [`RESUME_LIMIT`] (a long video: Glyph stays paused for
//!   good), and not when the user pressed play meanwhile.

use crate::controller::Core;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Longer than this, Glyph stays paused afterwards.
pub const RESUME_LIMIT: Duration = Duration::from_secs(2 * 60);
/// The other sound must be quiet this long (the gap between two videos,
/// an ad, buffering) before Glyph plays on.
pub const QUIET_FOR: Duration = Duration::from_secs(4);

const ACTIVE: u8 = 1;
const MUSIC: u8 = 2;
/// The platform is sure the other sound plays (not just a guess that it
/// will): only then does its end count as "over".
const SURE: u8 = 4;

/// What the platform saw last (bits ACTIVE, MUSIC, SURE).
static SEEN: AtomicU8 = AtomicU8::new(0);
static STATE: Mutex<Watch> = Mutex::new(Watch::new());

/// From the platform: another app plays sound now (`active`), and whether
/// that is a music app (`music`), and whether that is sure (`sure`).
pub fn report(active: bool, music: bool, sure: bool) {
    let bits = if active { ACTIVE | if music { MUSIC } else { 0 } | if sure { SURE } else { 0 } } else { 0 };
    SEEN.store(bits, Ordering::Relaxed);
}

/// What to do now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Pause,
    Resume,
}

/// The rules, without the app around them (so they can be tested).
#[derive(Debug)]
pub struct Watch {
    /// The other sound played at the last step.
    was_active: bool,
    /// Glyph paused for it, at this time.
    paused_at: Option<Instant>,
    /// It was a music app: no resume.
    music: bool,
    /// Since when the other sound is quiet again.
    quiet_since: Option<Instant>,
    /// The other sound was surely seen playing: its end can be trusted.
    sure: bool,
}

impl Watch {
    pub const fn new() -> Self {
        Self { was_active: false, paused_at: None, music: false, quiet_since: None, sure: false }
    }

    /// One step. `enabled`: the setting; `resume`: play on afterwards;
    /// `sure`: the other sound surely plays (see [`report`]);
    /// `playing`: Glyph plays (here, not on another device) right now.
    #[allow(clippy::too_many_arguments)]
    pub fn step(&mut self, now: Instant, enabled: bool, resume: bool, active: bool, music: bool, sure: bool, playing: bool) -> Action {
        if !enabled {
            *self = Self::new();
            return Action::None;
        }
        let started = active && !self.was_active;
        self.was_active = active;
        if started {
            self.quiet_since = None;
            if playing {
                self.paused_at = Some(now);
                self.music = music;
                self.sure = sure;
                return Action::Pause;
            }
            return Action::None;
        }
        let Some(at) = self.paused_at else { return Action::None };
        if playing {
            // The user pressed play: their choice, nothing to undo later.
            self.paused_at = None;
            return Action::None;
        }
        if now.duration_since(at) > RESUME_LIMIT {
            self.paused_at = None;
            return Action::None;
        }
        if active {
            self.music |= music;
            self.sure |= sure;
            self.quiet_since = None;
            return Action::None;
        }
        let since = *self.quiet_since.get_or_insert(now);
        if now.duration_since(since) < QUIET_FOR {
            return Action::None;
        }
        self.paused_at = None;
        self.quiet_since = None;
        let ok = resume && !self.music && self.sure;
        self.sure = false;
        if ok { Action::Resume } else { Action::None }
    }
}

impl Default for Watch {
    fn default() -> Self {
        Self::new()
    }
}

/// From the platform's tick (a few times a second).
pub fn tick(core: &Arc<Core>) {
    let seen = SEEN.load(Ordering::Relaxed);
    let (enabled, resume) = {
        let st = core.st();
        (st.config.playback.pause_for_others, st.config.playback.resume_after_others)
    };
    // Sound on another device or another Glyph is none of the phone's business.
    let here = !crate::account::controlling(core) && !crate::jam::remote_on();
    let status = core.player_dto().status;
    let playing = status == "playing";
    let action = STATE.lock().unwrap_or_else(|e| e.into_inner()).step(
        Instant::now(),
        enabled && here,
        resume,
        seen & ACTIVE != 0,
        seen & MUSIC != 0,
        seen & SURE != 0,
        playing,
    );
    match action {
        Action::Pause if playing => {
            tracing::info!("other audio: another app plays, pausing");
            core.toggle_play();
        }
        Action::Resume if status == "paused" => {
            tracing::info!("other audio: the other app stopped, playing on");
            core.toggle_play();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(w: &mut Watch, t: Instant, active: bool, music: bool, playing: bool) -> Action {
        w.step(t, true, true, active, music, true, playing)
    }

    #[test]
    fn pauses_and_plays_on_after_a_video() {
        let t = Instant::now();
        let mut w = Watch::new();
        assert_eq!(s(&mut w, t, false, false, true), Action::None);
        assert_eq!(s(&mut w, t, true, false, true), Action::Pause);
        assert_eq!(s(&mut w, t + Duration::from_secs(30), true, false, false), Action::None);
        // Quiet, but not long enough yet.
        assert_eq!(s(&mut w, t + Duration::from_secs(60), false, false, false), Action::None);
        assert_eq!(s(&mut w, t + Duration::from_secs(62), false, false, false), Action::None);
        assert_eq!(s(&mut w, t + Duration::from_secs(64), false, false, false), Action::Resume);
        assert_eq!(s(&mut w, t + Duration::from_secs(65), false, false, true), Action::None);
    }

    #[test]
    fn a_short_gap_between_videos_does_not_resume() {
        let t = Instant::now();
        let mut w = Watch::new();
        s(&mut w, t, false, false, true);
        assert_eq!(s(&mut w, t, true, false, true), Action::Pause);
        assert_eq!(s(&mut w, t + Duration::from_secs(10), false, false, false), Action::None);
        assert_eq!(s(&mut w, t + Duration::from_secs(11), true, false, false), Action::None);
        assert_eq!(s(&mut w, t + Duration::from_secs(13), false, false, false), Action::None);
        assert_eq!(s(&mut w, t + Duration::from_secs(16), false, false, false), Action::None);
        assert_eq!(s(&mut w, t + Duration::from_secs(17), false, false, false), Action::Resume);
    }

    #[test]
    fn music_apps_and_long_videos_do_not_resume() {
        let t = Instant::now();
        let mut w = Watch::new();
        s(&mut w, t, false, false, true);
        assert_eq!(s(&mut w, t, true, true, true), Action::Pause);
        s(&mut w, t + Duration::from_secs(5), false, false, false);
        assert_eq!(s(&mut w, t + Duration::from_secs(9), false, false, false), Action::None);

        let mut w = Watch::new();
        s(&mut w, t, false, false, true);
        assert_eq!(s(&mut w, t, true, false, true), Action::Pause);
        s(&mut w, t + Duration::from_secs(3 * 60), true, false, false);
        s(&mut w, t + Duration::from_secs(3 * 60 + 1), false, false, false);
        assert_eq!(s(&mut w, t + Duration::from_secs(3 * 60 + 10), false, false, false), Action::None);
    }

    #[test]
    fn user_choices_win() {
        let t = Instant::now();
        // Play pressed while the video runs: not paused again.
        let mut w = Watch::new();
        s(&mut w, t, false, false, true);
        assert_eq!(s(&mut w, t, true, false, true), Action::Pause);
        assert_eq!(s(&mut w, t + Duration::from_secs(5), true, false, true), Action::None);
        assert_eq!(s(&mut w, t + Duration::from_secs(9), false, false, false), Action::None);
        assert_eq!(s(&mut w, t + Duration::from_secs(12), false, false, false), Action::None);

        // Other sound already playing when Glyph starts: nothing happens.
        let mut w = Watch::new();
        assert_eq!(s(&mut w, t, true, false, false), Action::None);
        assert_eq!(s(&mut w, t + Duration::from_secs(1), true, false, true), Action::None);

        // Setting off: nothing.
        let mut w = Watch::new();
        assert_eq!(w.step(t, false, true, true, false, true, true), Action::None);
    }

    #[test]
    fn unsure_sound_does_not_resume() {
        // Focus gone, but the other player was never seen: stay paused.
        let t = Instant::now();
        let mut w = Watch::new();
        w.step(t, true, true, false, false, false, true);
        assert_eq!(w.step(t, true, true, true, false, false, true), Action::Pause);
        assert_eq!(w.step(t + Duration::from_secs(5), true, true, false, false, false, false), Action::None);
        assert_eq!(w.step(t + Duration::from_secs(20), true, true, false, false, false, false), Action::None);

        // Seen later on: its end counts.
        let mut w = Watch::new();
        w.step(t, true, true, false, false, false, true);
        assert_eq!(w.step(t, true, true, true, false, false, true), Action::Pause);
        assert_eq!(w.step(t + Duration::from_secs(6), true, true, true, false, true, false), Action::None);
        assert_eq!(w.step(t + Duration::from_secs(30), true, true, false, false, false, false), Action::None);
        assert_eq!(w.step(t + Duration::from_secs(35), true, true, false, false, false, false), Action::Resume);
    }
}
