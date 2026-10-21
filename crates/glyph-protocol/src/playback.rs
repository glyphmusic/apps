//! Account-scoped remote playback. Presence and commands expire on the server.
use crate::{Device, Track};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub acknowledgements: bool,
    pub track: Option<Track>,
    pub playing: bool,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub volume: f32,
    /// Whether this app can accept playback commands (disabled during a jam).
    pub available: bool,
}

impl State {
    /// Whether `next` is worth telling the other devices: another song,
    /// play/pause, volume or availability, or a jump of the position (more
    /// than 3 s off from where `self` would be after `elapsed_ms`).
    pub fn differs(&self, next: &State, elapsed_ms: u64) -> bool {
        let id = |s: &State| s.track.as_ref().map(|t| (t.source, t.id.clone()));
        if id(self) != id(next)
            || self.playing != next.playing
            || self.available != next.available
            || self.acknowledgements != next.acknowledgements
            || self.duration_ms != next.duration_ms
            || (self.volume - next.volume).abs() > 0.005
        {
            return true;
        }
        let expected = if self.playing { self.position_ms.saturating_add(elapsed_ms) } else { self.position_ms };
        expected.abs_diff(next.position_ms) > 3_000
    }
}

/// Messages on `/v1/devices/playback/live` (JSON, tagged by `t`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Live {
    /// Server → app: a command waits for this device; poll now.
    Wake,
    /// App → server: this app's own state, sent when it [`State::differs`].
    Report { state: State },
    /// Server → apps: another device of the account changed.
    State { device: String, state: State },
    /// Server → apps: a device's live line closed (it may still poll).
    Gone { device: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Output {
    pub device: Device,
    pub online: bool,
    pub state: State,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Command {
    Transfer {
        tracks: Vec<Track>,
        index: usize,
        position_ms: u64,
        playing: bool,
    },
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    Seek {
        position_ms: u64,
    },
    Volume {
        volume: f32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Send {
    #[serde(default)]
    pub request_id: Option<String>,
    pub target: String,
    pub command: Command,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Poll {
    pub devices: Vec<Output>,
    pub commands: Vec<Command>,
    #[serde(default)]
    pub deliveries: Vec<Delivery>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delivery {
    pub id: String,
    pub command: Command,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Receipt {
    pub id: String,
    pub done: bool,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playing(position_ms: u64) -> State {
        State { playing: true, position_ms, volume: 0.5, available: true, ..State::default() }
    }

    #[test]
    fn position_moving_on_is_no_change() {
        assert!(!playing(10_000).differs(&playing(15_200), 5_000));
        assert!(playing(10_000).differs(&playing(40_000), 5_000));
        let mut paused = playing(10_000);
        paused.playing = false;
        assert!(playing(10_000).differs(&paused, 0));
        assert!(!paused.differs(&paused.clone(), 60_000));
    }

    #[test]
    fn live_messages_are_tagged() {
        assert_eq!(serde_json::to_string(&Live::Wake).unwrap(), r#"{"t":"wake"}"#);
        let gone: Live = serde_json::from_str(r#"{"t":"gone","device":"d1"}"#).unwrap();
        assert!(matches!(gone, Live::Gone { device } if device == "d1"));
    }
}
