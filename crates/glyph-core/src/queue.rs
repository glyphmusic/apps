//! Play queue: "play next", "add to queue", reorder, remove, shuffle, repeat.

use glyph_protocol::Track;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

#[derive(Debug, Clone, Default)]
pub struct Queue {
    // implementation detail
}

impl Queue {
    pub fn new() -> Self {
        todo!()
    }
    /// Replace queue with `tracks`, start at `index`.
    pub fn set(&mut self, tracks: Vec<Track>, index: usize) {
        let _ = (tracks, index);
        todo!()
    }
    pub fn current(&self) -> Option<&Track> {
        todo!()
    }
    /// Upcoming tracks in play order (respects shuffle).
    pub fn upcoming(&self) -> Vec<&Track> {
        todo!()
    }
    pub fn play_next(&mut self, track: Track) {
        let _ = track;
        todo!()
    }
    pub fn push(&mut self, track: Track) {
        let _ = track;
        todo!()
    }
    /// Indices into `upcoming()`.
    pub fn move_item(&mut self, from: usize, to: usize) {
        let _ = (from, to);
        todo!()
    }
    pub fn remove(&mut self, upcoming_index: usize) {
        let _ = upcoming_index;
        todo!()
    }
    /// Advance (auto-next or user). Honors repeat; `None` = queue finished
    /// (UI may then start radio).
    pub fn next(&mut self) -> Option<&Track> {
        todo!()
    }
    pub fn previous(&mut self) -> Option<&Track> {
        todo!()
    }
    pub fn peek_next(&self) -> Option<&Track> {
        todo!()
    }
    pub fn shuffle(&self) -> bool {
        todo!()
    }
    pub fn set_shuffle(&mut self, on: bool) {
        let _ = on;
        todo!()
    }
    pub fn repeat(&self) -> RepeatMode {
        todo!()
    }
    pub fn set_repeat(&mut self, mode: RepeatMode) {
        let _ = mode;
        todo!()
    }
}
