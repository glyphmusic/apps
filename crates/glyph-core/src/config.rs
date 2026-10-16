//! `config.toml` in the app config dir (`dirs::config_dir()/glyph/`).
//! API keys come from here or from env vars (`GLYPH_YOUTUBE_KEY`, ...), never from code.

use glyph_protocol::ProviderId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Empty string = direct mode.
    pub server_url: String,
    pub keys: ApiKeys,
    pub providers: ProvidersConfig,
    pub visuals: VisualsConfig,
    pub updates: UpdatesConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiKeys {
    pub youtube: Option<String>,
    pub jamendo_client_id: Option<String>,
    pub soundcloud_client_id: Option<String>,
    pub spotify_client_id: Option<String>,
    pub spotify_client_secret: Option<String>,
    pub lastfm: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProvidersConfig {
    pub enabled: Vec<ProviderId>,
    /// Full-source priority for the resolver.
    pub priority: Vec<ProviderId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VisualsConfig {
    /// 0.0..=1.0
    pub intensity: f32,
    pub ascii_covers: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdatesConfig {
    /// `owner/repo` on GitHub.
    pub repo: String,
    pub check_on_start: bool,
    pub auto_install: bool,
    /// "stable" | "prerelease"
    pub channel: String,
}

impl Default for ProvidersConfig {
    fn default() -> Self {
        use ProviderId::*;
        Self {
            enabled: ProviderId::ALL.to_vec(),
            priority: vec![Audius, Jamendo, SoundCloud, YouTube],
        }
    }
}

impl Default for VisualsConfig {
    fn default() -> Self {
        Self { intensity: 0.8, ascii_covers: false }
    }
}

impl Default for UpdatesConfig {
    fn default() -> Self {
        Self {
            repo: "OWNER/glyph".into(),
            check_on_start: true,
            auto_install: false,
            channel: "stable".into(),
        }
    }
}

impl Config {
    pub fn dir() -> PathBuf {
        todo!()
    }
    pub fn path() -> PathBuf {
        todo!()
    }
    /// Missing file = defaults (and writes nothing). Env vars override keys.
    pub fn load() -> anyhow::Result<Self> {
        todo!()
    }
    pub fn save(&self) -> anyhow::Result<()> {
        todo!()
    }
}
