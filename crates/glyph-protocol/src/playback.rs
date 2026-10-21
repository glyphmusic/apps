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
