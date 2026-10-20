//! Shareable settings: `glyph:settings:` + base64url(deflate(JSON)), or the
//! same JSON saved as a file.
//!
//! Carries the sections that also follow an account (see [`settings_sync`]):
//! language, sources, appearance, updates, integrations, playback with the
//! equalizer, and player modes, plus the user's own equalizer presets
//! ([`crate::eq`] format).
//! Never API keys, the server address or device-bound values
//! (output device, paths, cache size, volume), so a file from someone else
//! cannot point this app at their server or leak anyone's keys.
//!
//! [`settings_sync`]: crate::settings_sync

use crate::config::Config;
use crate::eq::EqPreset;
use crate::settings_sync::{self, SettingsApplied};
use crate::sync::SyncState;
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use glyph_protocol::{sync_kind as kind, SyncItem};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::io::{Read, Write};

pub const PREFIX: &str = "glyph:settings:";
/// Suggested file name for "Save as file".
pub const FILE_NAME: &str = "glyph-settings.json";
/// Section with the user's equalizer presets (a list of [`EqPreset`]).
const EQ_PRESETS: &str = "eq_presets";
/// Bigger codes / files are refused.
const MAX_JSON: usize = 256 * 1024;

#[derive(Serialize, Deserialize)]
struct Shared {
    /// Format version.
    glyph_settings: u8,
    #[serde(default)]
    app_version: String,
    sections: Map<String, Value>,
}

fn shared(cfg: &Config) -> Shared {
    let mut sections: Map<String, Value> = settings_sync::items(cfg, None)
        .into_iter()
        .filter(|((k, _), _)| k == kind::SETTING)
        .map(|((_, name), v)| (name, v))
        .collect();
    if !cfg.audio.eq_user_presets.is_empty() {
        sections.insert(EQ_PRESETS.into(), serde_json::to_value(&cfg.audio.eq_user_presets).unwrap_or_default());
    }
    Shared { glyph_settings: 1, app_version: env!("CARGO_PKG_VERSION").to_string(), sections }
}

/// The settings as a readable JSON file.
pub fn to_json(cfg: &Config) -> String {
    serde_json::to_string_pretty(&shared(cfg)).unwrap_or_default()
}

/// The settings as one line to paste anywhere.
pub fn encode(cfg: &Config) -> String {
    let json = serde_json::to_vec(&shared(cfg)).unwrap_or_default();
    let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    let _ = z.write_all(&json);
    let packed = z.finish().unwrap_or_default();
    format!("{PREFIX}{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(packed))
}

/// Text that looks like a settings code (whitespace around it is fine).
pub fn is_code(s: &str) -> bool {
    s.trim().starts_with(PREFIX)
}

/// Shared settings read from a code or a settings file.
pub struct Imported {
    sections: Vec<(String, Value)>,
}

/// Reads a settings code or the content of a settings file.
pub fn decode(s: &str) -> Result<Imported> {
    let s = s.trim();
    let json = if let Some(body) = s.strip_prefix(PREFIX) {
        let packed = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(body.trim_end_matches('='))
            .context("broken settings code")?;
        let mut json = Vec::new();
        flate2::read::DeflateDecoder::new(&packed[..])
            .take(MAX_JSON as u64 + 1)
            .read_to_end(&mut json)
            .context("broken settings code")?;
        json
    } else {
        s.as_bytes().to_vec()
    };
    if json.len() > MAX_JSON {
        return Err(anyhow!("settings too large"));
    }
    let shared: Shared = serde_json::from_slice(&json).map_err(|_| anyhow!("not Glyph settings"))?;
    if shared.glyph_settings != 1 {
        return Err(anyhow!("settings from a newer Glyph; update first"));
    }
    Ok(Imported { sections: shared.sections.into_iter().collect() })
}

impl Imported {
    /// Applies the shared sections to `cfg`; unknown or broken ones are skipped.
    /// Equalizer presets are added (same name: replaced), never removed.
    pub fn apply(&self, cfg: &mut Config) -> SettingsApplied {
        let before = cfg.audio.eq_user_presets.clone();
        let presets = self.sections.iter().find(|(n, _)| n == EQ_PRESETS).and_then(|(_, v)| serde_json::from_value::<Vec<EqPreset>>(v.clone()).ok());
        for p in presets.unwrap_or_default() {
            let p = EqPreset::new(&p.name, p.gains());
            if p.name.is_empty() {
                continue;
            }
            match cfg.audio.eq_user_presets.iter_mut().find(|q| q.name == p.name) {
                Some(q) => *q = p,
                None => cfg.audio.eq_user_presets.push(p),
            }
        }
        let items: Vec<SyncItem> = self
            .sections
            .iter()
            .filter(|(n, _)| n != EQ_PRESETS)
            .map(|(name, v)| SyncItem {
                kind: kind::SETTING.to_string(),
                key: name.clone(),
                value: Some(v.clone()),
                updated_at: 0,
                deleted: false,
                rev: 0,
            })
            .collect();
        let mut r = settings_sync::apply(cfg, &mut SyncState::default(), &items, None);
        r.changed |= cfg.audio.eq_user_presets != before;
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_and_file_round_trip_without_secrets() {
        let mut a = Config::default();
        a.language = "de".into();
        a.audio.eq_enabled = true;
        a.audio.eq_preset = "rock".into();
        a.audio.eq_gains = vec![4.0, 3.0, 2.0, 0.5, -1.0, -1.0, 0.5, 2.0, 3.0, 4.0];
        a.audio.eq_user_presets = vec![EqPreset::new("Car", [1.0; 10])];
        a.server_url = "https://mine.example".into();
        a.keys.lastfm = Some("lastfm-secret".into());
        a.file_keys = Some(a.keys.clone());

        let code = encode(&a);
        assert!(is_code(&format!(" {code}\n")));
        let file = to_json(&a);
        assert!(!file.contains("lastfm-secret") && !file.contains("mine.example"));

        for text in [code.as_str(), file.as_str()] {
            let mut b = Config::default();
            let r = decode(text).unwrap().apply(&mut b);
            assert!(r.changed);
            assert_eq!((b.language.as_str(), b.audio.eq_enabled, b.audio.eq_preset.as_str()), ("de", true, "rock"));
            assert_eq!(b.audio.eq_gains, a.audio.eq_gains);
            assert_eq!(b.audio.eq_user_presets, a.audio.eq_user_presets);
            assert_eq!(b.server_url, "");
            assert!(b.keys.lastfm.is_none());
        }
        assert!(decode("glyph:settings:@@@").is_err());
        assert!(decode("{\"hello\": 1}").is_err());
    }
}
