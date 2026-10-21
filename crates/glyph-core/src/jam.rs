//! Jam decisions without UI or I/O, shared by the desktop and phone apps
//! (wire format: `glyph_protocol::jam`): clock sync with the server, how a
//! guest stays in step with the host, the order votes give the queue, and
//! when a guest takes over from a host that went offline.

use glyph_protocol::jam::{Member, HANDOVER_MS};
use std::collections::{HashMap, VecDeque};

/// Below this a guest counts as in step (ms).
pub const IN_STEP_MS: i64 = 60;
/// Up to this a guest catches up by playing a little faster or slower;
/// beyond it, it jumps (ms).
pub const NUDGE_MAX_MS: i64 = 1500;
/// Largest tempo change for catching up (2 %: hardly audible).
pub const MAX_NUDGE: f32 = 0.02;
/// A nudge closes the gap in about this long (ms).
const NUDGE_OVER_MS: f32 = 4000.0;
/// A jump lands this far ahead, since opening the stream there takes a
/// moment (ms).
pub const SEEK_LEAD_MS: i64 = 250;
/// Extra wait per guest in line before taking over (ms), so two guests do
/// not take over at once.
pub const HANDOVER_STEP_MS: u64 = 30_000;

/// Server clock estimate from timed requests (like NTP): each sample is the
/// server time seen between sending and receiving; the one with the
/// shortest round trip is the most exact.
#[derive(Debug, Clone, Default)]
pub struct ClockSync {
    /// (round trip ms, server minus local ms), newest last.
    samples: VecDeque<(u64, i64)>,
}

impl ClockSync {
    const KEEP: usize = 8;

    /// A request sent at local `sent_ms`, answered with `server_ms`,
    /// received at local `recv_ms`.
    pub fn add(&mut self, sent_ms: u64, server_ms: u64, recv_ms: u64) {
        if recv_ms < sent_ms {
            return;
        }
        let rtt = recv_ms - sent_ms;
        let mid = sent_ms as i64 + rtt as i64 / 2;
        self.samples.push_back((rtt, server_ms as i64 - mid));
        while self.samples.len() > Self::KEEP {
            self.samples.pop_front();
        }
    }

    /// Server minus local clock (ms), if measured.
    pub fn offset(&self) -> Option<i64> {
        self.samples.iter().min_by_key(|(rtt, _)| *rtt).map(|(_, o)| *o)
    }

    /// Shortest round trip seen (ms).
    pub fn best_rtt(&self) -> Option<u64> {
        self.samples.iter().map(|(rtt, _)| *rtt).min()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Samples kept.
    pub fn len(&self) -> usize {
        self.samples.len()
    }
}

/// What a guest does to stay with the host.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    /// In step: normal tempo.
    Keep,
    /// Play at this tempo for a while (slightly above or below 1.0).
    Nudge(f32),
    /// Jump to this position (ms).
    Seek(u64),
}

/// `have_ms`: where this device is; `want_ms`: where the host is now.
/// `can_nudge`: the player can change its tempo (native playback, not an
/// embedded player or a network device).
pub fn step(have_ms: u64, want_ms: u64, can_nudge: bool) -> Step {
    let gap = want_ms as i64 - have_ms as i64;
    let far = if can_nudge { NUDGE_MAX_MS } else { 700 };
    if gap.abs() <= IN_STEP_MS {
        Step::Keep
    } else if gap.abs() <= far && can_nudge {
        let rate = (gap as f32 / NUDGE_OVER_MS).clamp(-MAX_NUDGE, MAX_NUDGE);
        Step::Nudge(1.0 + rate)
    } else if gap.abs() <= far {
        // Small gaps without tempo control: live with them, a jump would be
        // worse than the gap.
        Step::Keep
    } else {
        Step::Seek((want_ms as i64 + SEEK_LEAD_MS).max(0) as u64)
    }
}

/// The queue's song keys in the order votes give them: most votes first,
/// otherwise as they were (stable).
pub fn vote_order(keys: &[String], votes: &HashMap<String, usize>) -> Vec<String> {
    let mut out = keys.to_vec();
    out.sort_by_key(|k| std::cmp::Reverse(votes.get(k).copied().unwrap_or(0)));
    out
}

/// Should `me` take over a jam whose host has been offline for
/// `offline_ms`? Guests take turns in the order they joined: the first after
/// [`HANDOVER_MS`], the next one [`HANDOVER_STEP_MS`] later, and so on, in
/// case the ones before are gone too.
pub fn should_take_over(members: &[Member], me: &str, offline_ms: u64) -> bool {
    let Some(place) = members.iter().filter(|m| !m.host).position(|m| m.handle == me) else { return false };
    offline_ms >= HANDOVER_MS + place as u64 * HANDOVER_STEP_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_uses_the_fastest_round_trip() {
        let mut c = ClockSync::default();
        assert_eq!(c.offset(), None);
        // Slow answer: server 5_000 seen between 1_000 and 1_400 → offset 3_800.
        c.add(1_000, 5_000, 1_400);
        // Fast answer: server 6_020 between 2_000 and 2_040 → offset 4_000.
        c.add(2_000, 6_020, 2_040);
        assert_eq!(c.offset(), Some(4_000));
        assert_eq!(c.best_rtt(), Some(40));
        // Clock going backwards is ignored.
        c.add(3_000, 1, 2_000);
        assert_eq!(c.offset(), Some(4_000));
    }

    #[test]
    fn clock_forgets_old_samples() {
        let mut c = ClockSync::default();
        c.add(0, 1_000, 2);
        for i in 0..ClockSync::KEEP as u64 {
            c.add(100 + i, 2_000, 150 + i);
        }
        assert_ne!(c.offset(), Some(999));
    }

    #[test]
    fn small_gaps_nudge_big_gaps_jump() {
        assert_eq!(step(10_000, 10_030, true), Step::Keep);
        match step(10_000, 10_400, true) {
            Step::Nudge(r) => assert!(r > 1.0 && r <= 1.0 + MAX_NUDGE),
            s => panic!("{s:?}"),
        }
        match step(10_400, 10_000, true) {
            Step::Nudge(r) => assert!(r < 1.0 && r >= 1.0 - MAX_NUDGE),
            s => panic!("{s:?}"),
        }
        assert_eq!(step(10_000, 20_000, true), Step::Seek(20_000 + SEEK_LEAD_MS as u64));
        // No tempo control: small gaps stay, larger ones jump.
        assert_eq!(step(10_000, 10_500, false), Step::Keep);
        assert_eq!(step(10_000, 11_000, false), Step::Seek(11_000 + SEEK_LEAD_MS as u64));
    }

    #[test]
    fn votes_move_songs_up_and_keep_ties_in_order() {
        let keys: Vec<String> = ["a", "b", "c", "d"].map(String::from).to_vec();
        let votes = HashMap::from([("c".to_string(), 2), ("d".to_string(), 1), ("b".to_string(), 1)]);
        assert_eq!(vote_order(&keys, &votes), ["c", "b", "d", "a"]);
        assert_eq!(vote_order(&keys, &HashMap::new()), keys);
    }

    #[test]
    fn guests_take_over_one_after_another() {
        let m = |h: &str, host: bool| Member { handle: h.into(), host, avatar: None };
        let members = vec![m("host", true), m("ana", false), m("ben", false)];
        assert!(!should_take_over(&members, "ana", HANDOVER_MS - 1));
        assert!(should_take_over(&members, "ana", HANDOVER_MS));
        assert!(!should_take_over(&members, "ben", HANDOVER_MS));
        assert!(should_take_over(&members, "ben", HANDOVER_MS + HANDOVER_STEP_MS));
        assert!(!should_take_over(&members, "host", u64::MAX / 2));
        assert!(!should_take_over(&members, "nobody", u64::MAX / 2));
    }
}
