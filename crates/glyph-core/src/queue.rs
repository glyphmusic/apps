//! Play queue: "play next", "add to queue", reorder, remove, shuffle, repeat.
//!
//! Two parts, like a streaming app's queue:
//! * the *context*: the list playback started from (album, playlist, search
//!   results). `base` holds its tracks in their original order, `order` is the
//!   play order (indices into `base`; identity when shuffle is off), `pos` the
//!   position of the context's current track inside `order`.
//! * `manual`: songs the user queued by hand ("Up next"). They play before the
//!   rest of the context, in the order they were added, and survive starting
//!   another list. While one of them plays, `manual_current` holds it and
//!   `pos` still points at the context track that played before it.
//!
//! Indices in [`Queue::upcoming`] (and everything taking an "upcoming index")
//! count the manual songs first, then the context.

use glyph_protocol::Track;
use rand::seq::SliceRandom;
use rand::Rng;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

/// How shuffle picks the play order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShuffleMode {
    /// Feels random: songs of one artist are spread over the whole list and
    /// songs played lately come last (the "dithering" approach Spotify
    /// described in 2014: true randomness clusters, and clusters feel wrong).
    #[default]
    Smart,
    /// Every order is equally likely (Fisher-Yates).
    Random,
}

#[derive(Debug, Clone, Default)]
pub struct Queue {
    base: Vec<Track>,
    order: Vec<usize>,
    pos: Option<usize>,
    shuffle: bool,
    shuffle_mode: ShuffleMode,
    repeat: RepeatMode,
    manual: Vec<Track>,
    manual_current: Option<Track>,
    /// Song keys played lately (smart shuffle puts them last).
    recent: Vec<String>,
    /// Repeat all with shuffle: the order of the next round, dealt while
    /// the last song of this one plays (see `prepare_round`).
    next_round: Option<Vec<usize>>,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the context with `tracks`, start at `index`. Songs queued by
    /// hand stay queued.
    pub fn set(&mut self, tracks: Vec<Track>, index: usize) {
        self.base = tracks;
        self.order = (0..self.base.len()).collect();
        self.manual_current = None;
        if self.base.is_empty() {
            self.pos = None;
            return;
        }
        let index = index.min(self.base.len() - 1);
        if self.shuffle {
            let rest: Vec<usize> = (0..self.base.len()).filter(|i| *i != index).collect();
            let rest = self.shuffled(rest);
            self.order = std::iter::once(index).chain(rest).collect();
            self.pos = Some(0);
        } else {
            self.pos = Some(index);
        }
        self.prepare_round();
    }

    /// Replace the context with `tracks`, shuffle on, every song dealt
    /// (the first one too). Returns the song to start with.
    pub fn set_shuffled(&mut self, tracks: Vec<Track>) -> Option<&Track> {
        self.shuffle = true;
        self.base = tracks;
        self.manual_current = None;
        let all: Vec<usize> = (0..self.base.len()).collect();
        self.order = self.shuffled(all);
        self.pos = (!self.base.is_empty()).then_some(0);
        self.current()
    }

    /// Shows someone else's queue (a jam guest follows the host's): `manual`
    /// as "Up next", then `after`, nothing current. Going on from here plays
    /// them in this order.
    pub fn mirror(&mut self, manual: Vec<Track>, after: Vec<Track>) {
        self.manual = manual;
        self.manual_current = None;
        self.order = (0..after.len()).collect();
        self.base = after;
        self.pos = None;
    }

    /// Context and hand-queued songs both emptied.
    pub fn clear(&mut self) {
        self.base.clear();
        self.order.clear();
        self.pos = None;
        self.manual.clear();
        self.manual_current = None;
    }

    /// Songs in the context (the list playback started from).
    pub fn len(&self) -> usize {
        self.base.len()
    }

    pub fn is_empty(&self) -> bool {
        self.base.is_empty() && self.manual.is_empty() && self.manual_current.is_none()
    }

    pub fn current(&self) -> Option<&Track> {
        if let Some(t) = &self.manual_current {
            return Some(t);
        }
        self.pos.map(|p| &self.base[self.order[p]])
    }

    /// The current song was queued by hand (not part of the context).
    pub fn current_is_manual(&self) -> bool {
        self.manual_current.is_some()
    }

    /// Already played context tracks (oldest first) in play order.
    pub fn history(&self) -> Vec<&Track> {
        let end = match (self.pos, &self.manual_current) {
            (Some(p), Some(_)) => p + 1,
            (Some(p), None) => p,
            (None, _) => 0,
        };
        self.order[..end].iter().map(|&i| &self.base[i]).collect()
    }

    fn upcoming_start(&self) -> usize {
        self.pos.map(|p| p + 1).unwrap_or(0)
    }

    /// Songs queued by hand that are still to come ("Up next").
    pub fn manual_len(&self) -> usize {
        self.manual.len()
    }

    /// Upcoming tracks in play order: the hand-queued songs, then the rest
    /// of the context (respects shuffle).
    pub fn upcoming(&self) -> Vec<&Track> {
        self.manual.iter().chain(self.order[self.upcoming_start()..].iter().map(|&i| &self.base[i])).collect()
    }

    /// Next in the context only (without the hand-queued songs).
    pub fn upcoming_context(&self) -> Vec<&Track> {
        self.order[self.upcoming_start()..].iter().map(|&i| &self.base[i]).collect()
    }

    fn insert_base(&mut self, at: usize, track: Track) -> usize {
        let at = at.min(self.base.len());
        self.base.insert(at, track);
        for o in &mut self.order {
            if *o >= at {
                *o += 1;
            }
        }
        at
    }

    /// Adds songs to the end of the context, after everything still to come
    /// (radio going on after a list ended). History stays as it is.
    pub fn append(&mut self, tracks: Vec<Track>) {
        let start = self.base.len();
        self.base.extend(tracks);
        self.order.extend(start..self.base.len());
    }

    /// Plays right after the current song (first of "Up next").
    pub fn play_next(&mut self, track: Track) {
        self.manual.insert(0, track);
    }

    /// Adds to the end of "Up next" (still before the rest of the context).
    pub fn push(&mut self, track: Track) {
        self.manual.push(track);
    }

    /// Removes context entry `ci` (index into the context's upcoming part).
    fn take_context(&mut self, ci: usize) -> Option<Track> {
        let at = self.upcoming_start() + ci;
        if at >= self.order.len() {
            return None;
        }
        let bi = self.order.remove(at);
        let t = self.base.remove(bi);
        for o in &mut self.order {
            if *o > bi {
                *o -= 1;
            }
        }
        Some(t)
    }

    /// Inserts `track` at context upcoming index `ci`.
    fn put_context(&mut self, ci: usize, track: Track) {
        let start = self.upcoming_start();
        let bi = self.insert_base(self.base.len(), track);
        let at = (start + ci).min(self.order.len());
        self.order.insert(at, bi);
    }

    /// Indices into `upcoming()`. A song may move between "Up next" and the
    /// context.
    pub fn move_item(&mut self, from: usize, to: usize) {
        let total = self.manual.len() + self.order.len().saturating_sub(self.upcoming_start());
        if from >= total || total == 0 {
            return;
        }
        let to = to.min(total - 1);
        if from == to {
            return;
        }
        let was_manual = from < self.manual.len();
        let item = if was_manual { Some(self.manual.remove(from)) } else { self.take_context(from - self.manual.len()) };
        let Some(item) = item else { return };
        // Right on the border between the parts the song stays where it was.
        let m = self.manual.len();
        if to < m || (to == m && was_manual) {
            self.manual.insert(to.min(m), item);
        } else {
            self.put_context(to - m, item);
        }
    }

    pub fn remove(&mut self, upcoming_index: usize) {
        if upcoming_index < self.manual.len() {
            self.manual.remove(upcoming_index);
        } else {
            self.take_context(upcoming_index - self.manual.len());
        }
    }

    /// Next context position (wraps with repeat).
    fn next_ctx_pos(&self) -> Option<usize> {
        if self.order.is_empty() {
            return None;
        }
        let next = self.upcoming_start();
        if next < self.order.len() {
            Some(next)
        } else if self.repeat == RepeatMode::All || (self.repeat == RepeatMode::One && self.pos.is_some()) {
            Some(0)
        } else {
            None
        }
    }

    fn advance(&mut self) -> Option<&Track> {
        if !self.manual.is_empty() {
            self.manual_current = Some(self.manual.remove(0));
            return self.manual_current.as_ref();
        }
        let p = self.next_ctx_pos()?;
        if self.upcoming_start() >= self.order.len() {
            // Repeat all wraps: the next round's order, when one was dealt.
            if let Some(round) = self.next_round.take().filter(|r| r.len() == self.order.len()) {
                self.order = round;
            }
        }
        self.manual_current = None;
        self.pos = Some(p);
        self.prepare_round();
        self.current()
    }

    /// Repeat all with shuffle, on the last song of a round: deals the next
    /// round now (a new order, not opened by the song that ends this one),
    /// so [`Queue::peek_next`] already knows its first song.
    fn prepare_round(&mut self) {
        let last_song = self.pos.is_some_and(|p| p + 1 >= self.order.len());
        if !(self.shuffle && self.repeat == RepeatMode::All && last_song && self.order.len() > 1) {
            self.next_round = None;
            return;
        }
        if self.next_round.as_ref().is_some_and(|r| r.len() == self.order.len()) {
            return;
        }
        let ending = self.pos.map(|p| self.order[p]);
        let mut round = self.shuffled((0..self.base.len()).collect());
        if round.first().copied() == ending {
            round.rotate_left(1);
        }
        self.next_round = Some(round);
    }

    /// Context index of the song after the current one in play order (the
    /// next round's first when repeat all wraps).
    fn next_ctx_index(&self) -> Option<usize> {
        let p = self.next_ctx_pos()?;
        let wraps = self.upcoming_start() >= self.order.len();
        Some(match &self.next_round {
            Some(r) if wraps && r.len() == self.order.len() => r[0],
            _ => self.order[p],
        })
    }

    /// Advance (auto-next or user). Honors repeat; `None` = queue finished
    /// (UI may then start radio). With `RepeatMode::One` this replays the
    /// current track — use [`Queue::skip_next`] for the user's "next" button.
    pub fn next(&mut self) -> Option<&Track> {
        if self.repeat == RepeatMode::One && self.current().is_some() {
            return self.current();
        }
        self.advance()
    }

    /// User-initiated skip: like `next()` but `RepeatMode::One` does not pin
    /// the current track.
    pub fn skip_next(&mut self) -> Option<&Track> {
        self.advance()
    }

    /// Previous track; at the very start returns the current track (restart),
    /// or wraps to the last one with `RepeatMode::All`. Leaving a hand-queued
    /// song puts it back at the front of "Up next".
    pub fn previous(&mut self) -> Option<&Track> {
        if let Some(t) = self.manual_current.take() {
            self.manual.insert(0, t);
            if self.pos.is_none() {
                // Nothing played before it: stay on it.
                self.manual_current = Some(self.manual.remove(0));
            }
            return self.current();
        }
        let p = self.pos?;
        if p > 0 {
            self.pos = Some(p - 1);
        } else if self.repeat == RepeatMode::All && self.order.len() > 1 {
            self.pos = Some(self.order.len() - 1);
        }
        self.prepare_round();
        self.current()
    }

    pub fn peek_next(&self) -> Option<&Track> {
        if self.repeat == RepeatMode::One && self.current().is_some() {
            return self.current();
        }
        if let Some(t) = self.manual.first() {
            return Some(t);
        }
        self.next_ctx_index().map(|i| &self.base[i])
    }

    /// Jump to an entry of `upcoming()`; it becomes the current track. A
    /// hand-queued song drops the ones queued before it; a context song keeps
    /// "Up next" (it plays after the picked song).
    pub fn jump_to_upcoming(&mut self, upcoming_index: usize) -> Option<&Track> {
        let m = self.manual.len();
        if upcoming_index < m {
            self.manual.drain(..upcoming_index);
            self.manual_current = Some(self.manual.remove(0));
            return self.manual_current.as_ref();
        }
        let at = self.upcoming_start() + (upcoming_index - m);
        if at >= self.order.len() {
            return None;
        }
        self.manual_current = None;
        self.pos = Some(at);
        self.prepare_round();
        self.current()
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn shuffle_mode(&self) -> ShuffleMode {
        self.shuffle_mode
    }

    pub fn set_shuffle_mode(&mut self, mode: ShuffleMode) {
        if self.shuffle_mode == mode {
            return;
        }
        self.shuffle_mode = mode;
        if self.shuffle {
            // Re-deal what is still to come with the new mode.
            let start = self.upcoming_start().min(self.order.len());
            let rest = self.order.split_off(start);
            let rest = self.shuffled(rest);
            self.order.extend(rest);
        }
    }

    /// Song keys played lately; smart shuffle deals them last.
    pub fn set_recent(&mut self, keys: Vec<String>) {
        self.recent = keys;
    }

    /// `idx` (indices into `base`) in shuffled order.
    fn shuffled(&self, mut idx: Vec<usize>) -> Vec<usize> {
        let mut rng = rand::thread_rng();
        match self.shuffle_mode {
            ShuffleMode::Random => {
                idx.shuffle(&mut rng);
                idx
            }
            ShuffleMode::Smart => smart_shuffle(&self.base, idx, &self.recent, &mut rng),
        }
    }

    pub fn set_shuffle(&mut self, on: bool) {
        if on == self.shuffle {
            return;
        }
        self.shuffle = on;
        if self.order.is_empty() {
            return;
        }
        match self.pos {
            Some(p) => {
                let cur = self.order[p];
                if on {
                    // Keep history and current, shuffle what is still to come.
                    let rest = self.order.split_off(p + 1);
                    let rest = self.shuffled(rest);
                    self.order.extend(rest);
                } else {
                    self.order = (0..self.base.len()).collect();
                    self.pos = Some(cur);
                }
            }
            None => {
                if on {
                    let all = std::mem::take(&mut self.order);
                    self.order = self.shuffled(all);
                } else {
                    self.order = (0..self.base.len()).collect();
                }
            }
        }
        self.prepare_round();
    }

    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }

    pub fn set_repeat(&mut self, mode: RepeatMode) {
        self.repeat = mode;
        self.prepare_round();
    }
}

/// Spreads songs of each artist evenly over the list: every artist's songs get
/// evenly spaced slots with a random start and a little jitter, then all
/// slots are sorted. Recently played songs and exact duplicates are pushed
/// towards the end so they do not come right back.
fn smart_shuffle(base: &[Track], idx: Vec<usize>, recent: &[String], rng: &mut impl Rng) -> Vec<usize> {
    use std::collections::HashMap;
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for i in idx {
        let a = crate::dedupe::primary_artist(&base[i].artist).to_lowercase();
        groups.entry(a).or_default().push(i);
    }
    let recent: std::collections::HashSet<&str> = recent.iter().map(String::as_str).collect();
    let mut slots: Vec<(f32, usize)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = Default::default();
    // Sorted for a deterministic walk; the randomness comes from `rng`.
    let mut groups: Vec<(String, Vec<usize>)> = groups.into_iter().collect();
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    for (_, mut g) in groups {
        g.shuffle(rng);
        let n = g.len() as f32;
        let gap = 1.0 / n;
        let start = rng.gen_range(0.0..gap);
        for (k, i) in g.into_iter().enumerate() {
            let jitter = rng.gen_range(-0.1..0.1) * gap;
            let mut at = start + k as f32 * gap + jitter;
            let key = crate::dedupe::track_key(&base[i]);
            // Slots spread over -0.1..1.1 (jitter), so the offsets must clear that range.
            if recent.contains(key.as_str()) {
                at += 2.0;
            }
            if !seen.insert(key) {
                // The same song twice in one list: the copy goes last.
                at += 4.0;
            }
            slots.push((at, i));
        }
    }
    slots.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    slots.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dedupe::test_track;
    use glyph_protocol::ProviderId;

    fn tr(n: &str) -> Track {
        test_track(ProviderId::Audius, n, n, "a", None)
    }

    fn q(names: &[&str], idx: usize) -> Queue {
        let mut q = Queue::new();
        q.set(names.iter().map(|n| tr(n)).collect(), idx);
        q
    }

    fn names(v: Vec<&Track>) -> Vec<String> {
        v.into_iter().map(|t| t.id.clone()).collect()
    }

    #[test]
    fn mirror_shows_a_queue_in_order() {
        let mut q = q(&["x", "y"], 0);
        q.push(tr("m"));
        q.mirror(vec![tr("a"), tr("b")], vec![tr("c")]);
        assert!(q.current().is_none());
        assert_eq!(q.manual_len(), 2);
        assert_eq!(names(q.upcoming()), ["a", "b", "c"]);
        assert_eq!(q.peek_next().unwrap().id, "a");
        assert_eq!(q.next().unwrap().id, "a");
        assert_eq!(names(q.upcoming()), ["b", "c"]);
        q.mirror(vec![], vec![]);
        assert!(q.next().is_none());
    }

    #[test]
    fn plays_through_and_finishes() {
        let mut q = q(&["a", "b", "c"], 0);
        assert_eq!(q.current().unwrap().id, "a");
        assert_eq!(names(q.upcoming()), ["b", "c"]);
        assert_eq!(q.peek_next().unwrap().id, "b");
        assert_eq!(q.next().unwrap().id, "b");
        assert_eq!(q.next().unwrap().id, "c");
        assert!(q.peek_next().is_none());
        assert!(q.next().is_none());
        assert_eq!(q.current().unwrap().id, "c");
    }

    #[test]
    fn repeat_modes() {
        let mut a = q(&["a", "b"], 1);
        a.set_repeat(RepeatMode::All);
        assert_eq!(a.next().unwrap().id, "a");
        assert_eq!(a.previous().unwrap().id, "b"); // wraps backwards
        let mut o = q(&["a", "b"], 0);
        o.set_repeat(RepeatMode::One);
        assert_eq!(o.next().unwrap().id, "a");
        assert_eq!(o.skip_next().unwrap().id, "b");
    }

    #[test]
    fn queued_songs_play_before_the_context() {
        let mut q = q(&["a", "b", "c"], 0);
        q.push(tr("x"));
        q.push(tr("y"));
        q.play_next(tr("z"));
        assert_eq!(q.manual_len(), 3);
        assert_eq!(names(q.upcoming()), ["z", "x", "y", "b", "c"]);
        assert_eq!(q.next().unwrap().id, "z");
        assert!(q.current_is_manual());
        assert_eq!(names(q.upcoming()), ["x", "y", "b", "c"]);
        // Back from a queued song: the context song before it, and the
        // queued one waits at the front again.
        assert_eq!(q.previous().unwrap().id, "a");
        assert_eq!(names(q.upcoming()), ["z", "x", "y", "b", "c"]);
        assert_eq!(q.next().unwrap().id, "z");
        assert_eq!(q.next().unwrap().id, "x");
        assert_eq!(q.next().unwrap().id, "y");
        assert_eq!(q.next().unwrap().id, "b");
        assert!(!q.current_is_manual());
    }

    #[test]
    fn repeat_all_reshuffles_every_round() {
        let all: Vec<&str> = vec!["a", "b", "c", "d", "e", "f", "g", "h"];
        let mut q = q(&all, 0);
        q.set_shuffle_mode(ShuffleMode::Random);
        q.set_shuffle(true);
        q.set_repeat(RepeatMode::All);
        let mut rounds: Vec<Vec<String>> = Vec::new();
        for _ in 0..6 {
            let mut round = vec![q.current().unwrap().id.clone()];
            for _ in 1..all.len() {
                round.push(q.next().unwrap().id.clone());
            }
            let last = round.last().cloned();
            let mut sorted = round.clone();
            sorted.sort();
            assert_eq!(sorted, all, "every song once per round");
            // Wraps into the next round, never with the song that just ended,
            // and with the song announced as next (gapless preload).
            let announced = q.peek_next().unwrap().id.clone();
            let first = q.next().unwrap().id.clone();
            assert_eq!(first, announced);
            assert_ne!(first, last.unwrap());
            rounds.push(round);
        }
        // Rounds 2.. start at the wrap; with 8! orders they are not all equal.
        assert!(rounds[1..].windows(2).any(|w| w[0] != w[1]));
    }

    #[test]
    fn appended_songs_follow_the_ended_list() {
        let mut q = q(&["a", "b"], 0);
        assert_eq!(q.next().unwrap().id, "b");
        assert!(q.peek_next().is_none());
        q.append(vec![tr("r1"), tr("r2")]);
        assert_eq!(q.skip_next().unwrap().id, "r1");
        assert_eq!(names(q.history()), ["a", "b"]);
        assert_eq!(names(q.upcoming()), ["r2"]);
        assert_eq!(q.previous().unwrap().id, "b");
        // After songs queued by hand, with nothing of the context playing.
        let mut m = Queue::new();
        m.push(tr("x"));
        assert_eq!(m.next().unwrap().id, "x");
        assert!(m.peek_next().is_none());
        m.append(vec![tr("r")]);
        assert_eq!(m.skip_next().unwrap().id, "r");
    }

    #[test]
    fn queue_survives_a_new_context() {
        let mut q = q(&["a", "b"], 0);
        q.push(tr("x"));
        q.set(vec![tr("c"), tr("d")], 0);
        assert_eq!(names(q.upcoming()), ["x", "d"]);
    }

    #[test]
    fn move_and_remove_across_sections() {
        let mut q = q(&["a", "b", "c"], 0);
        q.push(tr("x"));
        q.push(tr("z"));
        assert_eq!(names(q.upcoming()), ["x", "z", "b", "c"]);
        q.move_item(1, 0);
        assert_eq!(names(q.upcoming()), ["z", "x", "b", "c"]);
        q.move_item(3, 0);
        assert_eq!(names(q.upcoming()), ["c", "z", "x", "b"]);
        assert_eq!(q.manual_len(), 3);
        q.move_item(0, 3);
        assert_eq!(names(q.upcoming()), ["z", "x", "b", "c"]);
        assert_eq!(q.manual_len(), 2);
        q.remove(1);
        assert_eq!(names(q.upcoming()), ["z", "b", "c"]);
        q.remove(2);
        assert_eq!(names(q.upcoming()), ["z", "b"]);
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn empty_queue_starts_with_first_push() {
        let mut q = Queue::new();
        assert!(q.current().is_none() && q.next().is_none());
        q.push(tr("a"));
        assert_eq!(names(q.upcoming()), ["a"]);
        assert_eq!(q.next().unwrap().id, "a");
        assert!(q.upcoming().is_empty());
        // Nothing before it: previous stays on it.
        assert_eq!(q.previous().unwrap().id, "a");
    }

    #[test]
    fn shuffle_keeps_current_and_all_tracks() {
        for mode in [ShuffleMode::Smart, ShuffleMode::Random] {
            let mut q = q(&["a", "b", "c", "d", "e", "f"], 2);
            q.set_shuffle_mode(mode);
            q.set_shuffle(true);
            assert_eq!(q.current().unwrap().id, "c");
            let mut up = names(q.upcoming());
            up.sort();
            assert_eq!(up, ["d", "e", "f"].map(String::from));
            q.set_shuffle(false);
            assert_eq!(q.current().unwrap().id, "c");
            assert_eq!(names(q.upcoming()), ["d", "e", "f"]);
            assert_eq!(names(q.history()), ["a", "b"]);
        }
    }

    #[test]
    fn set_with_shuffle_starts_at_index() {
        let mut q = Queue::new();
        q.set_shuffle(true);
        q.set(["a", "b", "c", "d"].iter().map(|n| tr(n)).collect(), 3);
        assert_eq!(q.current().unwrap().id, "d");
        assert_eq!(q.upcoming().len(), 3);
    }

    #[test]
    fn smart_shuffle_spreads_artists() {
        // 4 songs by A, 4 by B: a smart order never has 3 of one artist in a row.
        let base: Vec<Track> = (0..8)
            .map(|i| test_track(ProviderId::Audius, &format!("s{i}"), &format!("s{i}"), if i < 4 { "A" } else { "B" }, None))
            .collect();
        let mut rng = rand::thread_rng();
        for _ in 0..200 {
            let order = smart_shuffle(&base, (0..8).collect(), &[], &mut rng);
            assert_eq!(order.len(), 8);
            let artists: Vec<&str> = order.iter().map(|&i| base[i].artist.as_str()).collect();
            assert!(!artists.windows(3).any(|w| w[0] == w[1] && w[1] == w[2]), "{artists:?}");
        }
    }

    #[test]
    fn smart_shuffle_deals_recent_songs_last() {
        let base: Vec<Track> = ["a", "b", "c", "d"].iter().map(|n| test_track(ProviderId::Audius, n, n, n, None)).collect();
        let recent = vec![crate::dedupe::track_key(&base[0])];
        let mut rng = rand::thread_rng();
        for _ in 0..50 {
            let order = smart_shuffle(&base, (0..4).collect(), &recent, &mut rng);
            assert_eq!(*order.last().unwrap(), 0);
        }
    }

    #[test]
    fn jump_to_upcoming() {
        let mut q = q(&["a", "b", "c", "d"], 0);
        assert_eq!(q.jump_to_upcoming(1).unwrap().id, "c");
        assert_eq!(names(q.upcoming()), ["d"]);
        assert!(q.jump_to_upcoming(5).is_none());
        q.push(tr("x"));
        q.push(tr("y"));
        assert_eq!(q.jump_to_upcoming(1).unwrap().id, "y");
        assert_eq!(names(q.upcoming()), ["d"]);
        q.push(tr("z"));
        assert_eq!(q.jump_to_upcoming(1).unwrap().id, "d");
        assert_eq!(names(q.upcoming()), ["z"]);
    }
}
