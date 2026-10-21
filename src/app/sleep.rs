//! Sleep timer: pauses playback after a chosen time or at the end of the
//! current song. Not saved: a restart forgets it.

use super::*;

/// Lengths offered in the menu, in minutes.
pub const SLEEP_MINUTES: [u32; 5] = [15, 30, 45, 60, 90];

impl Glyph {
    pub fn set_sleep(&mut self, sleep: Option<Sleep>, cx: &mut Context<Self>) {
        self.sleep = sleep;
        match sleep {
            Some(Sleep::At { mins, .. }) => self.toast_info(tf("sleep.toast.set", &[("time", &sleep_label(mins))]).to_string()),
            Some(Sleep::EndOfSong) => self.toast_info(t("sleep.toast.song")),
            None => self.toast_info(t("sleep.toast.off")),
        }
        cx.notify();
    }

    /// Starts a timer of `mins` minutes from now.
    pub fn sleep_in(&mut self, mins: u32, cx: &mut Context<Self>) {
        let until = Instant::now() + Duration::from_secs(mins as u64 * 60);
        self.set_sleep(Some(Sleep::At { until, mins }), cx);
    }

    /// Time left on a running timer (`None` for "end of song" or off).
    pub fn sleep_left(&self) -> Option<Duration> {
        match self.sleep {
            Some(Sleep::At { until, .. }) => Some(until.saturating_duration_since(Instant::now())),
            _ => None,
        }
    }

    pub(super) fn tick_sleep(&mut self, cx: &mut Context<Self>) {
        let Some(Sleep::At { until, .. }) = self.sleep else { return };
        if Instant::now() < until {
            return;
        }
        self.sleep = None;
        self.sleep_pause(cx);
    }

    /// The timer ran out: pause whatever plays (here, on a device, in a jam).
    fn sleep_pause(&mut self, cx: &mut Context<Self>) {
        if self.resolving {
            self.hold_play = true;
        } else if self.is_playing() {
            self.toggle_play(cx);
        }
        self.toast_info(t("sleep.toast.done"));
        cx.notify();
    }

    /// Called when a song ended on its own and the next one was started:
    /// with "end of song" set, that next song waits paused at its start.
    pub(super) fn sleep_after_song(&mut self, cx: &mut Context<Self>) {
        if self.sleep != Some(Sleep::EndOfSong) {
            return;
        }
        self.sleep = None;
        if !self.resolving && self.is_playing() {
            // Gapless start: the player already runs the next song.
            self.toggle_play(cx);
            self.seek_to(Duration::ZERO);
            self.toast_info(t("sleep.toast.done"));
            cx.notify();
        } else {
            self.sleep_pause(cx);
        }
    }
}

/// "15 min", "1 h", "1 h 30 min".
pub fn sleep_label(mins: u32) -> String {
    match (mins / 60, mins % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// Countdown text: "12:05", "1:02:05".
pub fn sleep_countdown(left: Duration) -> String {
    let s = left.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(sleep_label(15), "15 min");
        assert_eq!(sleep_label(60), "1 h");
        assert_eq!(sleep_label(90), "1 h 30 min");
        assert_eq!(sleep_countdown(Duration::from_secs(725)), "12:05");
        assert_eq!(sleep_countdown(Duration::from_secs(3725)), "1:02:05");
    }
}
