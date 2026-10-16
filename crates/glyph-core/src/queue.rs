//! Play queue: "play next", "add to queue", reorder, remove, shuffle, repeat.
//!
//! Internals: `base` holds the tracks in their original order, `order` is the
//! play order (indices into `base`; identity when shuffle is off), `pos` the
//! position of the current track inside `order`.

use glyph_protocol::Track;
use rand::seq::SliceRandom;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

#[derive(Debug, Clone, Default)]
pub struct Queue {
    base: Vec<Track>,
    order: Vec<usize>,
    pos: Option<usize>,
    shuffle: bool,
    repeat: RepeatMode,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace queue with `tracks`, start at `index`.
    pub fn set(&mut self, tracks: Vec<Track>, index: usize) {
        self.base = tracks;
        self.order = (0..self.base.len()).collect();
        if self.base.is_empty() {
            self.pos = None;
            return;
        }
        let index = index.min(self.base.len() - 1);
        if self.shuffle {
            let mut rest: Vec<usize> = (0..self.base.len()).filter(|i| *i != index).collect();
            rest.shuffle(&mut rand::thread_rng());
            self.order = std::iter::once(index).chain(rest).collect();
            self.pos = Some(0);
        } else {
            self.pos = Some(index);
        }
    }

    pub fn clear(&mut self) {
        self.base.clear();
        self.order.clear();
        self.pos = None;
    }

    pub fn len(&self) -> usize {
        self.base.len()
    }

    pub fn is_empty(&self) -> bool {
        self.base.is_empty()
    }

    pub fn current(&self) -> Option<&Track> {
        self.pos.map(|p| &self.base[self.order[p]])
    }

    /// Already played tracks (oldest first) in play order.
    pub fn history(&self) -> Vec<&Track> {
        match self.pos {
            Some(p) => self.order[..p].iter().map(|&i| &self.base[i]).collect(),
            None => vec![],
        }
    }

    fn upcoming_start(&self) -> usize {
        self.pos.map(|p| p + 1).unwrap_or(0)
    }

    /// Upcoming tracks in play order (respects shuffle).
    pub fn upcoming(&self) -> Vec<&Track> {
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

    pub fn play_next(&mut self, track: Track) {
        let start = self.upcoming_start();
        let base_at = match (self.shuffle, self.pos) {
            (false, Some(p)) => self.order[p] + 1,
            (false, None) => 0,
            (true, _) => self.base.len(),
        };
        let bi = self.insert_base(base_at, track);
        self.order.insert(start, bi);
    }

    pub fn push(&mut self, track: Track) {
        self.base.push(track);
        self.order.push(self.base.len() - 1);
    }

    /// Indices into `upcoming()`.
    pub fn move_item(&mut self, from: usize, to: usize) {
        let start = self.upcoming_start();
        let n = self.order.len() - start;
        if from >= n || n == 0 {
            return;
        }
        let to = to.min(n - 1);
        let v = self.order.remove(start + from);
        self.order.insert(start + to, v);
    }

    pub fn remove(&mut self, upcoming_index: usize) {
        let at = self.upcoming_start() + upcoming_index;
        if at >= self.order.len() {
            return;
        }
        let bi = self.order.remove(at);
        self.base.remove(bi);
        for o in &mut self.order {
            if *o > bi {
                *o -= 1;
            }
        }
    }

    fn next_pos(&self, honor_repeat_one: bool) -> Option<usize> {
        if self.order.is_empty() {
            return None;
        }
        if honor_repeat_one && self.repeat == RepeatMode::One {
            if let Some(p) = self.pos {
                return Some(p);
            }
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

    /// Advance (auto-next or user). Honors repeat; `None` = queue finished
    /// (UI may then start radio). With `RepeatMode::One` this replays the
    /// current track — use [`Queue::skip_next`] for the user's "next" button.
    pub fn next(&mut self) -> Option<&Track> {
        let p = self.next_pos(true)?;
        self.pos = Some(p);
        self.current()
    }

    /// User-initiated skip: like `next()` but `RepeatMode::One` does not pin
    /// the current track.
    pub fn skip_next(&mut self) -> Option<&Track> {
        let p = self.next_pos(false)?;
        self.pos = Some(p);
        self.current()
    }

    /// Previous track; at the very start returns the current track (restart),
    /// or wraps to the last one with `RepeatMode::All`.
    pub fn previous(&mut self) -> Option<&Track> {
        let p = self.pos?;
        if p > 0 {
            self.pos = Some(p - 1);
        } else if self.repeat == RepeatMode::All && self.order.len() > 1 {
            self.pos = Some(self.order.len() - 1);
        }
        self.current()
    }

    pub fn peek_next(&self) -> Option<&Track> {
        self.next_pos(true).map(|p| &self.base[self.order[p]])
    }

    /// Jump to an entry of `upcoming()`; it becomes the current track.
    pub fn jump_to_upcoming(&mut self, upcoming_index: usize) -> Option<&Track> {
        let at = self.upcoming_start() + upcoming_index;
        if at >= self.order.len() {
            return None;
        }
        self.pos = Some(at);
        self.current()
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
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
                    let start = p + 1;
                    self.order[start..].shuffle(&mut rand::thread_rng());
                } else {
                    self.order = (0..self.base.len()).collect();
                    self.pos = Some(cur);
                }
            }
            None => {
                if on {
                    self.order.shuffle(&mut rand::thread_rng());
                } else {
                    self.order = (0..self.base.len()).collect();
                }
            }
        }
    }

    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }

    pub fn set_repeat(&mut self, mode: RepeatMode) {
        self.repeat = mode;
    }
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
    fn play_next_push_move_remove() {
        let mut q = q(&["a", "b", "c"], 0);
        q.play_next(tr("x"));
        q.push(tr("z"));
        assert_eq!(names(q.upcoming()), ["x", "b", "c", "z"]);
        q.move_item(3, 0);
        assert_eq!(names(q.upcoming()), ["z", "x", "b", "c"]);
        q.remove(1);
        assert_eq!(names(q.upcoming()), ["z", "b", "c"]);
        assert_eq!(q.len(), 4);
        assert_eq!(q.next().unwrap().id, "z");
        assert_eq!(q.previous().unwrap().id, "a");
        assert_eq!(q.previous().unwrap().id, "a"); // start: restart
    }

    #[test]
    fn empty_queue_starts_with_first_push() {
        let mut q = Queue::new();
        assert!(q.current().is_none() && q.next().is_none());
        q.push(tr("a"));
        assert_eq!(names(q.upcoming()), ["a"]);
        assert_eq!(q.next().unwrap().id, "a");
        assert!(q.upcoming().is_empty());
    }

    #[test]
    fn shuffle_keeps_current_and_all_tracks() {
        let mut q = q(&["a", "b", "c", "d", "e", "f"], 2);
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

    #[test]
    fn set_with_shuffle_starts_at_index() {
        let mut q = Queue::new();
        q.set_shuffle(true);
        q.set(["a", "b", "c", "d"].iter().map(|n| tr(n)).collect(), 3);
        assert_eq!(q.current().unwrap().id, "d");
        assert_eq!(q.upcoming().len(), 3);
    }

    #[test]
    fn jump_to_upcoming() {
        let mut q = q(&["a", "b", "c", "d"], 0);
        assert_eq!(q.jump_to_upcoming(1).unwrap().id, "c");
        assert_eq!(names(q.upcoming()), ["d"]);
        assert!(q.jump_to_upcoming(5).is_none());
    }
}
