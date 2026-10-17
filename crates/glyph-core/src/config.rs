//! `config.toml` in the app config dir (`dirs::config_dir()/glyph/`).
//! API keys come from here or from env vars (`GLYPH_YOUTUBE_KEY`, ...), never from code.

use glyph_protocol::ProviderId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Public Glyph server used when `server_url` is empty, so nobody has to set
/// anything up. `server_url = "off"` switches to direct mode.
pub const DEFAULT_SERVER_URL: &str = "https://glyph-server.glyphmusic.workers.dev";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Empty = the public Glyph server ([`DEFAULT_SERVER_URL`]); "off" = direct mode.
    pub server_url: String,
    /// "" = system language; otherwise "en" | "de" | "ru".
    pub language: String,
    pub keys: ApiKeys,
    pub providers: ProvidersConfig,
    pub visuals: VisualsConfig,
    pub updates: UpdatesConfig,
    pub audio: AudioConfig,
    pub storage: StorageConfig,
    /// Keys as they stand in the file (before env overrides), so `save()` never
    /// writes env-provided secrets to disk.
    #[serde(skip)]
    #[doc(hidden)]
    pub file_keys: Option<ApiKeys>,
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
    /// Max YouTube `search.list` calls per day (each costs 100 of 10 000 quota units).
    pub youtube_daily_searches: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VisualsConfig {
    /// 0.0..=1.0
    pub intensity: f32,
    pub ascii_covers: bool,
    /// Particle animation that forms the logo at startup.
    pub startup_animation: bool,
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

/// Stream quality where a source offers several (Jamendo: mp31 / mp32 / ogg).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioQuality {
    /// Data saver (~96 kbps MP3).
    Low,
    /// ~128–160 kbps.
    Standard,
    /// Best the source offers.
    #[default]
    High,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// Seconds two songs overlap (0 = off, max 20).
    #[serde(default)]
    pub crossfade_s: u32,
    pub quality: AudioQuality,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// Cover cache limit in MB; 0 = no limit.
    pub cache_limit_mb: u64,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self { cache_limit_mb: 1024 }
    }
}

impl Default for ProvidersConfig {
    fn default() -> Self {
        use ProviderId::*;
        Self {
            // YouTube / SoundCloud (embed players) are opt-in fallbacks.
            enabled: ProviderId::ALL
                .into_iter()
                .filter(|p| !matches!(p, YouTube | SoundCloud))
                .collect(),
            priority: vec![Audius, Jamendo, SoundCloud, YouTube],
            youtube_daily_searches: 90,
        }
    }
}

impl Default for VisualsConfig {
    fn default() -> Self {
        Self { intensity: 0.8, ascii_covers: false, startup_animation: true }
    }
}

impl Default for UpdatesConfig {
    fn default() -> Self {
        Self {
            repo: "glyphmusic/glyph".into(),
            check_on_start: true,
            auto_install: false,
            channel: "stable".into(),
        }
    }
}

impl ApiKeys {
    fn blank_to_none(&mut self) {
        for k in [
            &mut self.youtube,
            &mut self.jamendo_client_id,
            &mut self.soundcloud_client_id,
            &mut self.spotify_client_id,
            &mut self.spotify_client_secret,
            &mut self.lastfm,
        ] {
            if k.as_deref().map(|s| s.trim().is_empty()).unwrap_or(false) {
                *k = None;
            }
        }
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

impl Config {
    /// UI language: the configured one, or the system language when empty;
    /// only "en" | "de" | "ru" are supported, anything else becomes "en".
    pub fn resolved_language(&self) -> &'static str {
        let configured = self.language.trim();
        if configured.is_empty() {
            Self::language_from_locale(sys_locale::get_locale().as_deref())
        } else {
            Self::language_from_locale(Some(configured))
        }
    }

    /// Maps a locale tag like "de-DE" / "ru_RU.UTF-8" to a supported language.
    pub fn language_from_locale(locale: Option<&str>) -> &'static str {
        let prefix = locale
            .unwrap_or("")
            .trim()
            .split(|c: char| c == '-' || c == '_' || c == '.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match prefix.as_str() {
            "de" => "de",
            "ru" => "ru",
            _ => "en",
        }
    }

    pub fn dir() -> PathBuf {
        dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("glyph")
    }

    pub fn path() -> PathBuf {
        if let Some(p) = env("GLYPH_CONFIG") {
            return PathBuf::from(p);
        }
        Self::dir().join("config.toml")
    }

    /// Missing file = defaults (and writes nothing). Env vars override keys.
    pub fn load() -> anyhow::Result<Self> {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &std::path::Path) -> anyhow::Result<Self> {
        let mut cfg = match std::fs::read_to_string(path) {
            Ok(text) => Self::from_toml(&text)
                .map_err(|e| anyhow::anyhow!("invalid {}: {e}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(e) => return Err(e.into()),
        };
        cfg.keys.blank_to_none();
        cfg.file_keys = Some(cfg.keys.clone());
        cfg.apply_env();
        Ok(cfg)
    }

    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    /// Applies `GLYPH_*` environment overrides (non-empty values only).
    pub fn apply_env(&mut self) {
        let k = &mut self.keys;
        if let Some(v) = env("GLYPH_YOUTUBE_KEY") {
            k.youtube = Some(v);
        }
        if let Some(v) = env("GLYPH_JAMENDO_CLIENT_ID") {
            k.jamendo_client_id = Some(v);
        }
        if let Some(v) = env("GLYPH_SOUNDCLOUD_CLIENT_ID") {
            k.soundcloud_client_id = Some(v);
        }
        if let Some(v) = env("GLYPH_SPOTIFY_CLIENT_ID") {
            k.spotify_client_id = Some(v);
        }
        if let Some(v) = env("GLYPH_SPOTIFY_CLIENT_SECRET") {
            k.spotify_client_secret = Some(v);
        }
        if let Some(v) = env("GLYPH_LASTFM_KEY") {
            k.lastfm = Some(v);
        }
        if let Some(v) = env("GLYPH_SERVER_URL") {
            self.server_url = v;
        }
    }

    /// The server actually used: the public one when none is set, nothing
    /// (direct mode) for "off".
    pub fn effective_server_url(&self) -> &str {
        match self.server_url.trim() {
            "" => DEFAULT_SERVER_URL,
            u if u.eq_ignore_ascii_case("off") || u.eq_ignore_ascii_case("direct") => "",
            u => u,
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let mut out = self.clone();
        // Do not persist keys that only exist because of an env var.
        if let Some(file_keys) = &self.file_keys {
            let mut env_cfg = Config::default();
            env_cfg.apply_env();
            let e = &env_cfg.keys;
            let f = file_keys;
            let k = &mut out.keys;
            macro_rules! keep {
                ($field:ident) => {
                    if e.$field.is_some() && k.$field == e.$field {
                        k.$field = f.$field.clone();
                    }
                };
            }
            keep!(youtube);
            keep!(jamendo_client_id);
            keep!(soundcloud_client_id);
            keep!(spotify_client_id);
            keep!(spotify_client_secret);
            keep!(lastfm);
        }
        let text = toml::to_string_pretty(&out)?;
        crate::paths::write_atomic(path, text.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_server_means_public_and_off_means_direct() {
        let mut c = Config::default();
        assert_eq!(c.effective_server_url(), DEFAULT_SERVER_URL);
        c.server_url = "OFF".into();
        assert_eq!(c.effective_server_url(), "");
        c.server_url = " https://mine.example ".into();
        assert_eq!(c.effective_server_url(), "https://mine.example");
    }

    #[test]
    fn defaults_roundtrip_through_toml() {
        let cfg = Config::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back = Config::from_toml(&text).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn partial_file_uses_defaults() {
        let cfg = Config::from_toml("server_url = \"https://x.example\"
[keys]
youtube = \"abc\"").unwrap();
        assert_eq!(cfg.server_url, "https://x.example");
        assert_eq!(cfg.keys.youtube.as_deref(), Some("abc"));
        assert!(cfg.updates.check_on_start);
        assert_eq!(cfg.providers.priority.len(), 4);
    }

    #[test]
    fn default_providers_exclude_embed_sources() {
        let e = Config::default().providers.enabled;
        assert!(!e.contains(&ProviderId::YouTube) && !e.contains(&ProviderId::SoundCloud));
        assert!(e.contains(&ProviderId::Audius) && e.contains(&ProviderId::LrcLib));
        // priority is unchanged; disabled providers are simply ignored by the resolver
        assert!(Config::default().providers.priority.contains(&ProviderId::YouTube));
    }

    #[test]
    fn language_resolution() {
        let mut c = Config::default();
        assert_eq!(Config::language_from_locale(Some("de-DE")), "de");
        assert_eq!(Config::language_from_locale(Some("ru_RU.UTF-8")), "ru");
        assert_eq!(Config::language_from_locale(Some("fr-FR")), "en");
        assert_eq!(Config::language_from_locale(None), "en");
        c.language = "ru".into();
        assert_eq!(c.resolved_language(), "ru");
        c.language = "xx".into();
        assert_eq!(c.resolved_language(), "en");
        c.language = String::new();
        assert!(["en", "de", "ru"].contains(&c.resolved_language()));
        assert_eq!(Config::from_toml("language = \"de\"").unwrap().language, "de");
    }

    #[test]
    fn missing_file_is_default_and_save_creates_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub").join("config.toml");
        let mut cfg = Config::load_from(&p).unwrap();
        assert!(!p.exists());
        cfg.keys.lastfm = Some("k".into());
        cfg.save_to(&p).unwrap();
        let again = Config::load_from(&p).unwrap();
        assert_eq!(again.keys.lastfm.as_deref(), Some("k"));
    }
}
