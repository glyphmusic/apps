//! `config.toml` in the app config dir (`dirs::config_dir()/glyph/`).
//! API keys come from here or from env vars (`GLYPH_YOUTUBE_KEY`, ...), never from code.

use glyph_protocol::ProviderId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Public Glyph server used when `server_url` is empty, so nobody has to set
/// anything up. `server_url = "off"` switches to direct mode.
pub const DEFAULT_SERVER_URL: &str = "https://api.glyph.my";
/// Earlier address of the public server; a saved copy of it means the default.
const OLD_SERVER_URL: &str = "https://glyph-server.glyphmusic.workers.dev";

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
    /// Player state kept across restarts (volume, shuffle, ...).
    pub player: PlayerPrefs,
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
    /// Where YouTube audio is resolved; kept local to this device.
    pub youtube_mode: YouTubeMode,
    pub youtube_only: bool,
    pub enabled: Vec<ProviderId>,
    /// Full-source priority for the resolver.
    pub priority: Vec<ProviderId>,
    /// Max YouTube `search.list` calls per day (each costs 100 of 10 000 quota units).
    pub youtube_daily_searches: u32,
    /// yt-dlp executable for local YouTube playback. `None` = `yt-dlp` on
    /// PATH, else the copy installed from settings (data dir).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ytdlp_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YouTubeMode {
    Local,
    #[default]
    Server,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VisualsConfig {
    /// 0.0..=1.0
    pub intensity: f32,
    pub music_reactivity: f32,
    pub ascii_covers: bool,
    /// Particle animation that forms the logo at startup.
    pub startup_animation: bool,
    /// ASCII effects: new text and pictures decode in, loading placeholders
    /// shimmer as ASCII. Off: things just appear, placeholders are plain boxes.
    pub ascii_effects: bool,
    /// Colour theme id (accent, background glow, base): see the app's
    /// `theme::THEMES`.
    pub theme: String,
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
    /// Choose conservatively from recent measured transfers.
    Auto,
    /// Data saver (~96 kbps MP3).
    Low,
    /// ~128–160 kbps.
    Standard,
    /// Best the source offers.
    #[default]
    High,
}

impl AudioQuality {
    pub fn effective(self) -> Self {
        if self != Self::Auto { return self; }
        match glyph_protocol::network::class() {
            glyph_protocol::network::Class::Slow => Self::Low,
            glyph_protocol::network::Class::Standard => Self::Standard,
            glyph_protocol::network::Class::Fast => Self::High,
        }
    }

    pub fn server_name(self) -> &'static str {
        match self.effective() {
            Self::Low => "low", Self::High => "high", _ => "standard",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// Seconds two songs overlap (0 = off, max 20).
    #[serde(default)]
    pub crossfade_s: u32,
    pub quality: AudioQuality,
    /// Quality of offline downloads (High = lossless where the source has it).
    pub download_quality: AudioQuality,
    /// Upper bound across all offline download batches (1..=10).
    pub download_parallelism: usize,
    /// Reduce simultaneous downloads on slow or unmeasured connections.
    pub download_adaptive: bool,
    /// Overall loudness target.
    pub level: VolumeLevel,
    /// Level every song to the same loudness (no jumps between songs).
    pub normalize: bool,
    /// Leave out songs marked explicit.
    pub hide_explicit: bool,
    /// Equalizer on, chosen preset (built-in key, `user:<name>`, or "custom"
    /// after manual changes) and the
    /// ten band gains in dB (32 Hz .. 16 kHz).
    pub eq_enabled: bool,
    pub eq_preset: String,
    pub eq_gains: Vec<f32>,
    /// Equalizer presets the user saved (selected as `user:<name>`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub eq_user_presets: Vec<crate::eq::EqPreset>,
    /// Output device name; None = system default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_device: Option<String>,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            crossfade_s: 0,
            quality: AudioQuality::default(),
            download_quality: AudioQuality::default(),
            download_parallelism: 5,
            download_adaptive: true,
            level: VolumeLevel::Normal,
            normalize: true,
            hide_explicit: false,
            eq_enabled: false,
            eq_preset: "flat".into(),
            eq_gains: vec![0.0; 10],
            eq_user_presets: Vec::new(),
            output_device: None,
        }
    }
}

/// Loud / Normal / Quiet (about -11 / -14 / -19 LUFS).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumeLevel {
    Loud,
    #[default]
    Normal,
    Quiet,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// Cover cache limit in MB; 0 = no limit.
    pub cache_limit_mb: u64,
    /// Data saver: small covers, feeds kept for hours, slower sync.
    pub data_saver: bool,
}

/// Player controls as the user left them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerPrefs {
    /// 0..=1.
    pub volume: f32,
    pub shuffle: bool,
    /// "off" | "all" | "one".
    pub repeat: String,
    /// Queue panel open.
    pub show_queue: bool,
    /// Mini player stays on top.
    pub mini_on_top: bool,
    /// Mini player as the big card (else the slim bar).
    pub mini_large: bool,
    /// Size of the card (the user can resize it freely).
    pub mini_card_w: f32,
    pub mini_card_h: f32,
}

impl Default for PlayerPrefs {
    fn default() -> Self {
        Self { volume: 1.0, shuffle: false, repeat: "off".into(), show_queue: false, mini_on_top: true, mini_large: false, mini_card_w: 300.0, mini_card_h: 384.0 }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self { cache_limit_mb: 1024, data_saver: false }
    }
}

impl Default for ProvidersConfig {
    fn default() -> Self {
        use ProviderId::*;
        Self {
            // YouTube first: with a local yt-dlp it plays full songs from this
            // machine (Deezer only has 30 s previews and often the wrong cut).
            // Without yt-dlp or an API key the YouTube provider simply is not
            // built. SoundCloud (embed player) stays opt-in.
            enabled: ProviderId::ALL.into_iter().filter(|p| !matches!(p, SoundCloud)).collect(),
            priority: vec![YouTube, Audius, Jamendo, SoundCloud],
            youtube_daily_searches: 90,
            ytdlp_path: None,
            youtube_mode: YouTubeMode::Server,
            youtube_only: false,
        }
    }
}

impl Default for VisualsConfig {
    fn default() -> Self {
        Self { intensity: 0.8, music_reactivity: 1.0, ascii_covers: false, startup_animation: true, ascii_effects: true, theme: "glyph".into() }
    }
}

/// Where the release workflow publishes (the app's own repository).
pub const RELEASES_REPO: &str = "glyphmusic/desktop";

impl Default for UpdatesConfig {
    fn default() -> Self {
        Self {
            repo: RELEASES_REPO.into(),
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
        if let Some(d) = crate::paths::demo_dir() {
            return d.join("config");
        }
        dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("glyph")
    }

    pub fn path() -> PathBuf {
        if crate::paths::demo_dir().is_none() {
            if let Some(p) = env("GLYPH_CONFIG") {
                return PathBuf::from(p);
            }
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
        // Early configs named a repository that has no releases.
        if cfg.updates.repo.trim() == "glyphmusic/glyph" {
            cfg.updates.repo = RELEASES_REPO.into();
        }
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
            u if u.trim_end_matches('/').eq_ignore_ascii_case(OLD_SERVER_URL) => DEFAULT_SERVER_URL,
            u if u.eq_ignore_ascii_case("off") || u.eq_ignore_ascii_case("direct") => "",
            u => u,
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&Self::path())
    }

    /// The user's own keys: what the file holds, never values that only exist
    /// because of an env var (those are neither saved nor synced).
    pub fn persisted_keys(&self) -> ApiKeys {
        let mut k = self.keys.clone();
        if let Some(f) = &self.file_keys {
            let mut env_cfg = Config::default();
            env_cfg.apply_env();
            let e = &env_cfg.keys;
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
        k
    }

    /// Replace the user's own keys (env overrides still win at runtime).
    pub fn set_persisted_keys(&mut self, keys: ApiKeys) {
        let mut keys = keys;
        keys.blank_to_none();
        self.file_keys = Some(keys.clone());
        self.keys = keys;
        let url = std::mem::take(&mut self.server_url);
        self.apply_env();
        self.server_url = url;
    }

    pub fn save_to(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let mut out = self.clone();
        // Do not persist keys that only exist because of an env var.
        out.keys = self.persisted_keys();
        let text = toml::to_string_pretty(&out)?;
        crate::paths::write_atomic(path, text.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_queue_settings_are_toml_only_and_backwards_compatible() {
        let old = Config::from_toml("").unwrap();
        assert_eq!(old.audio.download_parallelism, 5);
        assert!(old.audio.download_adaptive);
        assert!(!old.providers.youtube_only);
        let edited = Config::from_toml("[audio]\ndownload_parallelism = 7\ndownload_adaptive = false\n[providers]\nyoutube_only = true\n").unwrap();
        assert_eq!(edited.audio.download_parallelism, 7);
        assert!(!edited.audio.download_adaptive);
        assert!(edited.providers.youtube_only);
    }

    #[test]
    fn youtube_mode_defaults_to_server_and_persists_local_choice() {
        assert_eq!(Config::from_toml("").unwrap().providers.youtube_mode, YouTubeMode::Server);
        let mut config = Config::default();
        config.providers.youtube_mode = YouTubeMode::Local;
        let encoded = toml::to_string(&config).unwrap();
        assert_eq!(Config::from_toml(&encoded).unwrap().providers.youtube_mode, YouTubeMode::Local);
    }

    #[test]
    fn empty_server_means_public_and_off_means_direct() {
        let mut c = Config::default();
        assert_eq!(c.effective_server_url(), DEFAULT_SERVER_URL);
        c.server_url = "OFF".into();
        assert_eq!(c.effective_server_url(), "");
        c.server_url = " https://mine.example ".into();
        assert_eq!(c.effective_server_url(), "https://mine.example");
        c.server_url = "https://glyph-server.glyphmusic.workers.dev/".into();
        assert_eq!(c.effective_server_url(), DEFAULT_SERVER_URL);
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
    fn default_providers_prefer_youtube() {
        let e = Config::default().providers.enabled;
        assert!(e.contains(&ProviderId::YouTube) && !e.contains(&ProviderId::SoundCloud));
        assert!(e.contains(&ProviderId::Audius) && e.contains(&ProviderId::LrcLib));
        // YouTube (local yt-dlp) first; disabled providers are ignored by the resolver
        assert_eq!(Config::default().providers.priority[0], ProviderId::YouTube);
        // Older files without [player] / download_quality still load.
        let c: Config = toml::from_str("[audio]\nquality = \"low\"\n").unwrap();
        assert_eq!(c.player, PlayerPrefs::default());
        assert_eq!(c.audio.download_quality, AudioQuality::High);
        assert_eq!(c.audio.quality, AudioQuality::Low);
    }

    #[test]
    fn legacy_glow_strength_keeps_music_response_default() {
        let mut cfg = Config::from_toml("[visuals]\nintensity = 0.42\n").unwrap();
        assert_eq!(cfg.visuals.intensity, 0.42);
        assert_eq!(cfg.visuals.music_reactivity, 1.0);
        cfg.visuals.music_reactivity = 0.0;
        let back = Config::from_toml(&toml::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(back.visuals.intensity, 0.42);
        assert_eq!(back.visuals.music_reactivity, 0.0);
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
