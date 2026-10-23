//! Settings and API keys that follow the account (see `docs/ACCOUNTS.md`).
//!
//! Settings sync as a few `setting` items, one per section, so a change to the
//! equalizer on one device and to the language on another do not overwrite
//! each other. Device-bound values stay local: server address, paths (yt-dlp,
//! downloads, cache size), output device, volume and the mini player's
//! window (always-on-top, size); its style (bar or card) syncs.
//!
//! API keys sync as one `secret` item, sealed with XChaCha20-Poly1305 under a
//! [`Vault`] derived from the library's data key (`e2e::DataKey::vault`), and
//! like every item end-to-end encrypted on top. They only sync while the
//! server cannot read the data key, i.e. on accounts with a password. Copies
//! from older versions were sealed with a password-derived key; a device that
//! still has that key opens them once and pushes them in the new form.

use crate::config::{ApiKeys, AudioQuality, Config, IntegrationsConfig, UpdatesConfig, VisualsConfig, VolumeLevel};
use crate::sync::{Key, SyncState};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use glyph_protocol::{sync_kind as kind, ProviderId, SyncItem};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Key of the one encrypted item.
pub const API_KEYS: &str = "api_keys";

pub fn secret_key() -> Key {
    (kind::SECRET.to_string(), API_KEYS.to_string())
}

fn setting(name: &str) -> Key {
    (kind::SETTING.to_string(), name.to_string())
}

#[derive(Serialize, Deserialize)]
struct General {
    language: String,
}

#[derive(Serialize, Deserialize)]
struct Providers {
    enabled: Vec<ProviderId>,
    priority: Vec<ProviderId>,
    youtube_daily_searches: u32,
}

#[derive(Serialize, Deserialize)]
struct Audio {
    crossfade_s: u32,
    quality: AudioQuality,
    download_quality: AudioQuality,
    level: VolumeLevel,
    normalize: bool,
    hide_explicit: bool,
    eq_enabled: bool,
    eq_preset: String,
    eq_gains: Vec<f32>,
}

#[derive(Serialize, Deserialize)]
struct Player {
    shuffle: bool,
    repeat: String,
    show_queue: bool,
    /// Mini player as the card (`None`: sent by an older version).
    #[serde(default)]
    mini_large: Option<bool>,
}

fn json<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or_default()
}

/// Encryption key for the synced API keys.
#[derive(Clone, PartialEq, Eq)]
pub struct Vault([u8; 32]);

impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Vault(..)")
    }
}

impl Vault {
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        <[u8; 32]>::try_from(b).ok().map(Self)
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }

    /// Deterministic (nonce from key + plaintext): the same keys give the same
    /// item, so an unchanged value is never pushed again.
    pub fn seal(&self, plain: &[u8]) -> Value {
        let mut h = Sha256::new();
        h.update(self.0);
        h.update(b"glyph-nonce-v1");
        h.update(plain);
        let nonce = h.finalize();
        let nonce = XNonce::from_slice(&nonce[..24]);
        let ct = XChaCha20Poly1305::new((&self.0).into()).encrypt(nonce, plain).unwrap_or_default();
        serde_json::json!({ "v": 1, "nonce": b64(nonce), "ct": b64(&ct) })
    }

    /// `None`: wrong key or damaged item.
    pub fn open(&self, v: &Value) -> Option<Vec<u8>> {
        if v["v"].as_u64() != Some(1) {
            return None;
        }
        let nonce = unb64(v["nonce"].as_str()?)?;
        let ct = unb64(v["ct"].as_str()?)?;
        if nonce.len() != 24 {
            return None;
        }
        XChaCha20Poly1305::new((&self.0).into()).decrypt(XNonce::from_slice(&nonce), ct.as_slice()).ok()
    }
}

fn b64(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn unb64(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s).ok()
}

/// The synced parts of `cfg`. The API keys only with a vault.
pub fn items(cfg: &Config, vault: Option<&Vault>) -> Vec<(Key, Value)> {
    let a = &cfg.audio;
    let mut v = vec![
        (setting("general"), json(&General { language: cfg.language.clone() })),
        (
            setting("providers"),
            json(&Providers {
                enabled: cfg.providers.enabled.clone(),
                priority: cfg.providers.priority.clone(),
                youtube_daily_searches: cfg.providers.youtube_daily_searches,
            }),
        ),
        (setting("visuals"), json(&cfg.visuals)),
        (setting("updates"), json(&cfg.updates)),
        (setting("integrations"), json(&cfg.integrations)),
        (setting("stats"), json(&cfg.stats)),
        (setting("playback"), json(&cfg.playback)),
        (setting("lists"), json(&cfg.lists)),
        (
            setting("audio"),
            json(&Audio {
                crossfade_s: a.crossfade_s,
                quality: a.quality,
                download_quality: a.download_quality,
                level: a.level,
                normalize: a.normalize,
                hide_explicit: a.hide_explicit,
                eq_enabled: a.eq_enabled,
                eq_preset: a.eq_preset.clone(),
                eq_gains: a.eq_gains.clone(),
            }),
        ),
        (
            setting("player"),
            json(&Player {
                shuffle: cfg.player.shuffle,
                repeat: cfg.player.repeat.clone(),
                show_queue: cfg.player.show_queue,
                mini_large: Some(cfg.player.mini_large),
            }),
        ),
    ];
    if let Some(vault) = vault {
        let plain = serde_json::to_vec(&cfg.persisted_keys()).unwrap_or_default();
        v.push((secret_key(), vault.seal(&plain)));
    }
    v
}

fn parse<T: for<'de> Deserialize<'de>>(v: &Value) -> Option<T> {
    serde_json::from_value(v.clone()).ok()
}

/// What applying remote settings did.
#[derive(Debug, Default, PartialEq)]
pub struct SettingsApplied {
    /// `cfg` changed (save it and apply it to the running app).
    pub changed: bool,
    /// Keys or sources changed: providers need rebuilding.
    pub providers_changed: bool,
    /// Keys arrived that no key on this device opens (left alone).
    pub locked: bool,
}

/// Apply remote `setting` / `secret` items to `cfg` and record the result as
/// agreed. `legacy` opens keys sealed by older versions; those are not
/// agreed, so the next push replaces them with the current form.
pub fn apply(cfg: &mut Config, state: &mut SyncState, items_in: &[SyncItem], vault: Option<&Vault>, legacy: Option<&Vault>) -> SettingsApplied {
    let before = cfg.clone();
    let mut out = SettingsApplied::default();
    let mut touched: Vec<Key> = Vec::new();
    for i in items_in {
        let key = (i.kind.clone(), i.key.clone());
        let Some(v) = i.value.as_ref().filter(|_| !i.deleted) else {
            // Settings are never deleted; a tombstone just ends the agreement.
            if i.kind == kind::SETTING || i.kind == kind::SECRET {
                state.forget(&key);
            }
            continue;
        };
        match (i.kind.as_str(), i.key.as_str()) {
            (kind::SETTING, "general") => {
                let Some(g) = parse::<General>(v) else { continue };
                cfg.language = g.language;
            }
            (kind::SETTING, "providers") => {
                let Some(p) = parse::<Providers>(v) else { continue };
                cfg.providers.enabled = p.enabled;
                cfg.providers.priority = p.priority;
                cfg.providers.youtube_daily_searches = p.youtube_daily_searches;
            }
            (kind::SETTING, "visuals") => {
                let Some(x) = parse::<VisualsConfig>(v) else { continue };
                cfg.visuals = x;
            }
            (kind::SETTING, "updates") => {
                let Some(x) = parse::<UpdatesConfig>(v) else { continue };
                cfg.updates = x;
            }
            (kind::SETTING, "integrations") => {
                let Some(x) = parse::<IntegrationsConfig>(v) else { continue };
                cfg.integrations = x;
            }
            (kind::SETTING, "stats") => {
                let Some(x) = parse::<crate::config::StatsConfig>(v) else { continue };
                cfg.stats = x;
            }
            (kind::SETTING, "playback") => {
                let Some(x) = parse::<crate::config::PlaybackConfig>(v) else { continue };
                cfg.playback = x;
            }
            (kind::SETTING, "lists") => {
                let Some(x) = parse::<crate::config::ListsConfig>(v) else { continue };
                cfg.lists = x;
            }
            (kind::SETTING, "audio") => {
                let Some(x) = parse::<Audio>(v) else { continue };
                let a = &mut cfg.audio;
                a.crossfade_s = x.crossfade_s.min(20);
                a.quality = x.quality;
                a.download_quality = x.download_quality;
                a.level = x.level;
                a.normalize = x.normalize;
                a.hide_explicit = x.hide_explicit;
                a.eq_enabled = x.eq_enabled;
                a.eq_preset = x.eq_preset;
                if x.eq_gains.len() == 10 {
                    a.eq_gains = x.eq_gains.into_iter().map(|g| g.clamp(-12.0, 12.0)).collect();
                }
            }
            (kind::SETTING, "player") => {
                let Some(x) = parse::<Player>(v) else { continue };
                cfg.player.shuffle = x.shuffle;
                cfg.player.repeat = x.repeat;
                cfg.player.show_queue = x.show_queue;
                if let Some(big) = x.mini_large {
                    cfg.player.mini_large = big;
                }
            }
            (kind::SECRET, API_KEYS) => {
                let open = |k: Option<&Vault>| k.and_then(|k| k.open(v)).and_then(|p| serde_json::from_slice::<ApiKeys>(&p).ok());
                if let Some(keys) = open(vault) {
                    cfg.set_persisted_keys(keys);
                } else {
                    // Not agreed either way: an old copy is pushed again in
                    // the current form; one nothing here opens is neither
                    // overwritten nor deleted by this device.
                    match open(legacy) {
                        Some(keys) => cfg.set_persisted_keys(keys),
                        None => out.locked = true,
                    }
                    state.forget(&key);
                    continue;
                }
            }
            // Sections from newer app versions are left alone.
            _ => continue,
        }
        touched.push(key);
    }
    let after = items(cfg, vault);
    for key in touched {
        let v = after.iter().find(|(k, _)| *k == key).map(|(_, v)| v);
        state.agree(&key, v);
    }
    out.changed = *cfg != before;
    out.providers_changed = cfg.providers != before.providers || cfg.persisted_keys() != before.persisted_keys();
    out
}

/// Open keys that arrived while this device had no (current) vault.
pub fn restore_local_preferences(cfg: &mut Config, source: &Config) {
    let wire: Vec<_> = items(source, None).into_iter().map(|((kind, key), value)| SyncItem {
        kind, key, value: Some(value), updated_at: 0, deleted: false, rev: 0,
    }).collect();
    apply(cfg, &mut SyncState::default(), &wire, None, None);
    cfg.set_persisted_keys(source.persisted_keys());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: &Key, v: Value) -> SyncItem {
        SyncItem { kind: key.0.clone(), key: key.1.clone(), value: Some(v), updated_at: 1, deleted: false, rev: 1 }
    }

    fn vault(seed: u8) -> Vault {
        Vault([seed; 32])
    }

    #[test]
    fn sections_roundtrip_and_device_values_stay() {
        let mut a = Config::default();
        a.language = "de".into();
        a.audio.eq_enabled = true;
        a.audio.output_device = Some("Headphones".into());
        a.player.volume = 0.3;
        a.player.shuffle = true;
        a.server_url = "https://mine.example".into();
        a.providers.ytdlp_path = Some("C:/yt-dlp.exe".into());
        a.providers.youtube_mode = crate::config::YouTubeMode::Server;
        a.storage.cache_limit_mb = 50;

        let mut b = Config::default();
        b.player.volume = 0.9;
        let mut st = SyncState::default();
        let remote: Vec<SyncItem> = items(&a, None).iter().map(|(k, v)| item(k, v.clone())).collect();
        let r = apply(&mut b, &mut st, &remote, None, None);
        assert!(r.changed && !r.locked);
        assert_eq!((b.language.as_str(), b.audio.eq_enabled, b.player.shuffle), ("de", true, true));
        // Device-bound values are untouched.
        assert_eq!(b.player.volume, 0.9);
        assert_eq!(b.audio.output_device, None);
        assert_eq!(b.server_url, "");
        assert_eq!(b.providers.ytdlp_path, None);
        assert_eq!(b.providers.youtube_mode, crate::config::YouTubeMode::Local);
        assert_eq!(b.storage.cache_limit_mb, Config::default().storage.cache_limit_mb);
        // Agreed: nothing to push back.
        let snap = crate::sync::Snapshot { items: items(&b, None), ..Default::default() };
        assert!(st.changes(&snap, 2).is_empty());
    }

    #[test]
    fn keys_are_encrypted_and_need_the_same_vault() {
        let mut a = Config::default();
        a.keys.lastfm = Some("lastfm-secret".into());
        a.file_keys = Some(a.keys.clone());
        let sealed = items(&a, Some(&vault(1)));
        let (_, blob) = sealed.iter().find(|(k, _)| *k == secret_key()).unwrap();
        assert!(!blob.to_string().contains("lastfm-secret"));
        // Same input, same item: no needless pushes.
        assert_eq!(items(&a, Some(&vault(1))), sealed);

        // Right vault: keys arrive.
        let mut b = Config::default();
        let mut st = SyncState::default();
        let r = apply(&mut b, &mut st, &[item(&secret_key(), blob.clone())], Some(&vault(1)), None);
        assert!(r.providers_changed && !r.locked);
        assert_eq!(b.keys.lastfm.as_deref(), Some("lastfm-secret"));

        // No vault: left alone, nothing pushed or deleted.
        let mut c = Config::default();
        let mut st = SyncState::default();
        let r = apply(&mut c, &mut st, &[item(&secret_key(), blob.clone())], None, None);
        assert!(r.locked && c.keys.lastfm.is_none());
        let snap = crate::sync::Snapshot { items: items(&c, None), ..Default::default() };
        assert!(st.changes(&snap, 2).iter().all(|i| i.kind != kind::SECRET));

        // An old copy opens with the old key and goes up again in the new form.
        let mut d = Config::default();
        let mut st = SyncState::default();
        let r = apply(&mut d, &mut st, &[item(&secret_key(), blob.clone())], Some(&vault(3)), Some(&vault(1)));
        assert!(!r.locked);
        assert_eq!(d.keys.lastfm.as_deref(), Some("lastfm-secret"));
        let snap = crate::sync::Snapshot { items: items(&d, Some(&vault(3))), ..Default::default() };
        assert!(st.changes(&snap, 2).iter().any(|i| i.kind == kind::SECRET));
    }

    #[test]
    fn bad_remote_values_are_ignored() {
        let mut c = Config::default();
        let mut st = SyncState::default();
        let r = apply(&mut c, &mut st, &[item(&setting("audio"), serde_json::json!({"nonsense": 1})), item(&setting("future"), serde_json::json!({}))], None, None);
        assert!(!r.changed);
        assert_eq!(c, Config::default());
    }
}
