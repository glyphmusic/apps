//! End-to-end encryption of the account's synced items (see `docs/ACCOUNTS.md`,
//! "End-to-end encryption").
//!
//! Every account has one random data key. Synced items leave the device as
//! `e2e` / `e2e_playlist` items: the key is a keyed hash of the real
//! `(kind, key)` and the value is XChaCha20-Poly1305 ciphertext, so the server
//! sees neither songs nor names. The data key reaches other devices without
//! anything extra to remember:
//!
//! * wrapped with the account password ([`password_key`], never sent);
//! * sealed to a device's X25519 key (`env:<device>`), by a device that has it;
//! * through a pairing code ([`PairStart`]): SPAKE2 with a part of the code the
//!   server never sees;
//! * as `escrow` in the clear, only while the account has no password, so a
//!   lost device never loses the library (the transparency page says so).
//!
//! Nothing here talks to the network; `account.rs` drives it.

use anyhow::{anyhow, Result};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use glyph_protocol::{sync_kind as kind, vault_kdf, SyncItem};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Keyring entry with the wrapped data key (`{v, kid, pw?, escrow?}`).
pub const MAIN: &str = "main";
/// `dev:<device id>`: that device's public key, written by itself.
pub const DEV: &str = "dev:";
/// `env:<device id>`: the data key sealed to that device.
pub const ENV: &str = "env:";
/// `pair:<server part of the pairing code>`: the data key for a paired device.
pub const PAIR: &str = "pair:";

fn b64(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn unb64(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut b);
    b
}

fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut m = <HmacSha256 as Mac>::new_from_slice(key).expect("hmac takes any key length");
    for p in parts {
        m.update(&(p.len() as u32).to_le_bytes());
        m.update(p);
    }
    m.finalize().into_bytes().into()
}

fn hkdf(ikm: &[u8], salt: &[u8], info: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    hkdf::Hkdf::<Sha256>::new(Some(salt), ikm).expand(info.as_bytes(), &mut out).expect("32 bytes fit");
    out
}

fn seal(key: &[u8; 32], aad: &[u8], plain: &[u8]) -> Value {
    let nonce = random::<24>();
    let ct = XChaCha20Poly1305::new(key.into()).encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad }).unwrap_or_default();
    json!({ "n": b64(&nonce), "ct": b64(&ct) })
}

fn open(key: &[u8; 32], aad: &[u8], v: &Value) -> Option<Vec<u8>> {
    let nonce = unb64(v["n"].as_str()?)?;
    let ct = unb64(v["ct"].as_str()?)?;
    if nonce.len() != 24 {
        return None;
    }
    XChaCha20Poly1305::new(key.into()).decrypt(XNonce::from_slice(&nonce), Payload { msg: &ct, aad }).ok()
}

// ------------------------------------------------------------------ data key

/// The account's data key.
#[derive(Clone, PartialEq, Eq)]
pub struct DataKey([u8; 32]);

impl std::fmt::Debug for DataKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DataKey({})", self.kid())
    }
}

impl DataKey {
    pub fn generate() -> Self {
        Self(random())
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        <[u8; 32]>::try_from(b).ok().map(Self)
    }

    pub fn to_b64(&self) -> String {
        b64(&self.0)
    }

    pub fn from_b64(s: &str) -> Option<Self> {
        unb64(s).and_then(|b| Self::from_bytes(&b))
    }

    fn sub(&self, label: &str) -> [u8; 32] {
        hkdf(&self.0, b"glyph-data-key", label)
    }

    /// Public name of the key: tells which key encrypted an item, nothing more.
    pub fn kid(&self) -> String {
        hex(&self.sub("glyph-kid-v1")[..8])
    }

    /// Key of the encrypted item for `(kind, key)`.
    pub fn blind(&self, kind: &str, key: &str) -> String {
        hex(&hmac(&self.sub("glyph-index-v1"), &[kind.as_bytes(), key.as_bytes()]))
    }

    /// Key for the synced API keys (sealed once more inside their item).
    pub fn vault(&self) -> crate::settings_sync::Vault {
        crate::settings_sync::Vault::from_bytes(&self.sub("glyph-api-keys-v2")).expect("32 bytes")
    }

    /// Proof that a device public key was published by a device holding this key.
    pub fn device_mac(&self, device_id: &str, pk: &[u8]) -> String {
        hex(&hmac(&self.sub("glyph-device-v1"), &[device_id.as_bytes(), pk]))
    }

    /// Proof that a rotated key comes from a device that held this one.
    fn rotation_mac(&self, new_kid: &str, device_id: &str, epk: &str) -> String {
        hex(&hmac(&self.sub("glyph-rotate-v1"), &[new_kid.as_bytes(), device_id.as_bytes(), epk.as_bytes()]))
    }

    fn item_aad(enc_kind: &str, blinded: &str) -> Vec<u8> {
        format!("glyph-e2e-v1\0{enc_kind}\0{blinded}").into_bytes()
    }

    /// The encrypted form of a plain item (tombstones stay tombstones).
    pub fn seal_item(&self, item: &SyncItem) -> SyncItem {
        let enc_kind = if item.kind == kind::PLAYLIST { kind::E2E_PLAYLIST } else { kind::E2E };
        let blinded = self.blind(&item.kind, &item.key);
        let value = item.value.as_ref().filter(|_| !item.deleted).map(|v| {
            let plain = serde_json::to_vec(&json!({ "k": item.kind, "key": item.key, "v": v })).unwrap_or_default();
            let mut sealed = seal(&self.sub("glyph-item-v1"), &Self::item_aad(enc_kind, &blinded), &pad(&deflate(&plain)));
            sealed["v"] = json!(1);
            sealed["kid"] = json!(self.kid());
            sealed
        });
        SyncItem { kind: enc_kind.into(), key: blinded, deleted: value.is_none(), value, updated_at: item.updated_at, rev: item.rev }
    }

    /// The plain item inside an encrypted one. `Err` when it does not
    /// authenticate (changed on the way, or another key); tombstones need
    /// `lookup` (blinded key -> plain key) and are `Ok(None)` when unknown.
    pub fn open_item(&self, item: &SyncItem, lookup: impl Fn(&str) -> Option<(String, String)>) -> Result<Option<SyncItem>, OpenError> {
        if item.deleted || item.value.is_none() {
            return Ok(lookup(&item.key).map(|(k, key)| SyncItem { kind: k, key, value: None, updated_at: item.updated_at, deleted: true, rev: item.rev }));
        }
        let v = item.value.as_ref().unwrap();
        if v["kid"].as_str() != Some(self.kid().as_str()) {
            return Err(OpenError::OtherKey(v["kid"].as_str().unwrap_or_default().to_string()));
        }
        let plain = open(&self.sub("glyph-item-v1"), &Self::item_aad(&item.kind, &item.key), v)
            .and_then(|p| unpad(&p))
            .and_then(|p| inflate(&p))
            .ok_or(OpenError::Tampered)?;
        let inner: Value = serde_json::from_slice(&plain).map_err(|_| OpenError::Tampered)?;
        let (Some(k), Some(key)) = (inner["k"].as_str(), inner["key"].as_str()) else { return Err(OpenError::Tampered) };
        // The blinded key is part of the authenticated data; this keeps an
        // item from being presented under a different plain key.
        if self.blind(k, key) != item.key || (k == kind::PLAYLIST) != (item.kind == kind::E2E_PLAYLIST) {
            return Err(OpenError::Tampered);
        }
        Ok(Some(SyncItem { kind: k.into(), key: key.into(), value: Some(inner["v"].clone()), updated_at: item.updated_at, deleted: false, rev: item.rev }))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum OpenError {
    /// Encrypted with another data key (this kid).
    OtherKey(String),
    /// Does not authenticate: changed after it was encrypted.
    Tampered,
}

fn deflate(b: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = e.write_all(b);
    e.finish().unwrap_or_default()
}

fn inflate(b: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    // Bounded: an item is at most 1 MiB on the server.
    flate2::read::DeflateDecoder::new(b).take(16 * 1024 * 1024).read_to_end(&mut out).ok()?;
    Some(out)
}

/// Length prefix + zero padding, so sizes only tell a rough bucket.
fn pad(b: &[u8]) -> Vec<u8> {
    let len = b.len() + 4;
    let bucket = if len <= 4096 { len.next_power_of_two().max(256) } else { len.div_ceil(4096) * 4096 };
    let mut out = Vec::with_capacity(bucket);
    out.extend_from_slice(&(b.len() as u32).to_le_bytes());
    out.extend_from_slice(b);
    out.resize(bucket, 0);
    out
}

fn unpad(b: &[u8]) -> Option<Vec<u8>> {
    let len = u32::from_le_bytes(b.get(..4)?.try_into().ok()?) as usize;
    b.get(4..4 + len).map(<[u8]>::to_vec)
}

// ------------------------------------------------------------------ password

/// Key that wraps the data key with the account password. Argon2id with its
/// own parameters and secret ([`vault_kdf`]): no sign-in salt a server picks
/// can turn the sign-in key into this one.
pub fn password_key(password: &str, account_id: &str) -> Result<[u8; 32]> {
    use argon2::{Algorithm, Argon2, Params, Version};
    let salt = Sha256::digest(format!("{}{account_id}", vault_kdf::SALT_PREFIX).as_bytes());
    let params = Params::new(vault_kdf::M_KIB, vault_kdf::T_COST, vault_kdf::P_COST, Some(vault_kdf::KEY_LEN)).map_err(|e| anyhow!("argon2 params: {e}"))?;
    let mut out = [0u8; 32];
    Argon2::new_with_secret(vault_kdf::SECRET, Algorithm::Argon2id, Version::V0x13, params)
        .map_err(|e| anyhow!("argon2: {e}"))?
        .hash_password_into(password.as_bytes(), &salt[..16], &mut out)
        .map_err(|e| anyhow!("argon2: {e}"))?;
    Ok(out)
}

/// Salts that would make the sign-in key reveal a key used for data: the
/// old settings key (same parameters as sign-in) and the vault salt. A
/// server handing out one of them is trying to read the account's data.
pub fn suspicious_sign_in_salt(salt: &[u8], account_id: &str) -> bool {
    let v1 = Sha256::digest(format!("glyph-settings-v1:{account_id}").as_bytes());
    let v2 = Sha256::digest(format!("{}{account_id}", vault_kdf::SALT_PREFIX).as_bytes());
    salt == &v1[..16] || salt == &v2[..16]
}

fn pw_aad(account_id: &str, kid: &str) -> Vec<u8> {
    format!("glyph-keyring-pw-v1\0{account_id}\0{kid}").into_bytes()
}

/// `main.pw`: the data key wrapped with [`password_key`].
pub fn wrap_with_password(pw_key: &[u8; 32], account_id: &str, k: &DataKey) -> Value {
    seal(pw_key, &pw_aad(account_id, &k.kid()), &k.0)
}

pub fn unwrap_with_password(pw_key: &[u8; 32], account_id: &str, kid: &str, v: &Value) -> Option<DataKey> {
    open(pw_key, &pw_aad(account_id, kid), v).and_then(|b| DataKey::from_bytes(&b)).filter(|k| k.kid() == kid)
}

// ------------------------------------------------------------------ devices

/// This device's X25519 key for receiving the data key.
#[derive(Clone)]
pub struct DeviceKey(x25519_dalek::StaticSecret);

impl DeviceKey {
    pub fn generate() -> Self {
        Self(x25519_dalek::StaticSecret::random_from_rng(rand::rngs::OsRng))
    }

    pub fn from_b64(s: &str) -> Option<Self> {
        let b: [u8; 32] = unb64(s)?.try_into().ok()?;
        Some(Self(x25519_dalek::StaticSecret::from(b)))
    }

    pub fn to_b64(&self) -> String {
        b64(self.0.as_bytes())
    }

    pub fn public(&self) -> [u8; 32] {
        x25519_dalek::PublicKey::from(&self.0).to_bytes()
    }

    pub fn public_b64(&self) -> String {
        b64(&self.public())
    }
}

fn env_aad(device_id: &str, kid: &str) -> Vec<u8> {
    format!("glyph-env-v1\0{device_id}\0{kid}").into_bytes()
}

/// `env:<device>`: the data key sealed to that device's public key. With
/// `previous`, the entry also proves it comes from a device that held the
/// previous key (a rotation).
pub fn seal_to_device(device_id: &str, pk_b64: &str, k: &DataKey, by: &str, previous: Option<&DataKey>) -> Option<Value> {
    let pk: [u8; 32] = unb64(pk_b64)?.try_into().ok()?;
    let pk = x25519_dalek::PublicKey::from(pk);
    let eph = x25519_dalek::StaticSecret::random_from_rng(rand::rngs::OsRng);
    let epk = x25519_dalek::PublicKey::from(&eph);
    let shared = eph.diffie_hellman(&pk);
    if !shared.was_contributory() {
        return None;
    }
    let kek = hkdf(shared.as_bytes(), &[epk.as_bytes().as_slice(), pk.as_bytes().as_slice()].concat(), "glyph-env-v1");
    let mut v = seal(&kek, &env_aad(device_id, &k.kid()), &k.0);
    let epk = b64(epk.as_bytes());
    if let Some(p) = previous {
        v["rot"] = json!({ "from": p.kid(), "mac": p.rotation_mac(&k.kid(), device_id, &epk) });
    }
    v["epk"] = json!(epk);
    v["kid"] = json!(k.kid());
    v["by"] = json!(by);
    Some(v)
}

/// Opens an `env:` entry for this device.
pub fn open_from_device(device_id: &str, dk: &DeviceKey, v: &Value) -> Option<DataKey> {
    let kid = v["kid"].as_str()?;
    let epk: [u8; 32] = unb64(v["epk"].as_str()?)?.try_into().ok()?;
    let epk = x25519_dalek::PublicKey::from(epk);
    let shared = dk.0.diffie_hellman(&epk);
    if !shared.was_contributory() {
        return None;
    }
    let kek = hkdf(shared.as_bytes(), &[epk.as_bytes().as_slice(), dk.public().as_slice()].concat(), "glyph-env-v1");
    open(&kek, &env_aad(device_id, kid), v).and_then(|b| DataKey::from_bytes(&b)).filter(|k| k.kid() == kid)
}

/// The entry proves it was made by a device holding `previous`.
pub fn rotation_proven(v: &Value, device_id: &str, previous: &DataKey) -> bool {
    let (Some(kid), Some(epk)) = (v["kid"].as_str(), v["epk"].as_str()) else { return false };
    v["rot"]["from"].as_str() == Some(previous.kid().as_str()) && v["rot"]["mac"].as_str() == Some(previous.rotation_mac(kid, device_id, epk).as_str())
}

// ------------------------------------------------------------------ pairing

/// Characters of the pairing code (as the server's part).
pub const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
/// Characters only the two devices know.
pub const PAIR_SECRET_LEN: usize = 4;

fn pair_password(secret: &str) -> spake2::Password {
    spake2::Password::new(format!("glyph-pair-v1:{}", secret.to_ascii_uppercase()))
}

fn pair_identity() -> spake2::Identity {
    spake2::Identity::new(b"glyph-pair-v1")
}

/// `(server part, secret part)` of a typed pairing code. The secret part is
/// only there when the code has 12 characters.
pub fn split_pair_code(raw: &str) -> Option<(String, Option<String>)> {
    let c: String = raw.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect();
    if !c.bytes().all(|b| CODE_ALPHABET.contains(&b)) {
        return None;
    }
    match c.len() {
        8 => Some((format!("{}-{}", &c[..4], &c[4..]), None)),
        12 => Some((format!("{}-{}", &c[..4], &c[4..8]), Some(c[8..].to_string()))),
        _ => None,
    }
}

/// The new device's half of a pairing: keeps its SPAKE2 state until the
/// approving device answered.
pub struct PairStart {
    pub secret: String,
    /// First message, sent with the login start.
    pub msg: String,
    state: spake2::Spake2<spake2::Ed25519Group>,
}

impl PairStart {
    pub fn new() -> Self {
        let r = random::<PAIR_SECRET_LEN>();
        let secret: String = r.iter().map(|b| CODE_ALPHABET[(*b & 31) as usize] as char).collect();
        Self::with_secret(&secret)
    }

    pub fn with_secret(secret: &str) -> Self {
        let (state, msg) = spake2::Spake2::<spake2::Ed25519Group>::start_symmetric_with_rng(&pair_password(secret), &pair_identity(), rand::rngs::OsRng);
        Self { secret: secret.to_string(), msg: b64(&msg), state }
    }

    /// The entry was made for this pairing.
    pub fn is_for(&self, v: &Value) -> bool {
        v["for"].as_str() == Some(pair_tag(&self.msg).as_str())
    }

    /// The data key from the approving device's `pair:` entry. `Err` when the
    /// entry is for this pairing but does not open: somebody between the two
    /// devices changed a message.
    pub fn finish(self, v: &Value) -> Result<Option<DataKey>, OpenError> {
        if v["for"].as_str() != Some(pair_tag(&self.msg).as_str()) {
            return Ok(None);
        }
        let other = unb64(v["pake"].as_str().unwrap_or_default()).ok_or(OpenError::Tampered)?;
        let shared = self.state.finish(&other).map_err(|_| OpenError::Tampered)?;
        let kek = hkdf(&shared, b"glyph-pair", "glyph-pair-v1");
        let kid = v["kid"].as_str().unwrap_or_default();
        open(&kek, format!("glyph-pair-v1\0{kid}").as_bytes(), v).and_then(|b| DataKey::from_bytes(&b)).filter(|k| k.kid() == kid).map(Some).ok_or(OpenError::Tampered)
    }
}

impl Default for PairStart {
    fn default() -> Self {
        Self::new()
    }
}

fn pair_tag(msg_b64: &str) -> String {
    hex(&Sha256::digest(msg_b64.as_bytes())[..16])
}

/// The approving device's `pair:` entry: the data key for whoever knows the
/// code's secret part and sent `their_msg`.
pub fn pair_answer(secret: &str, their_msg: &str, k: &DataKey) -> Option<Value> {
    let (state, msg) = spake2::Spake2::<spake2::Ed25519Group>::start_symmetric_with_rng(&pair_password(secret), &pair_identity(), rand::rngs::OsRng);
    let shared = state.finish(&unb64(their_msg)?).ok()?;
    let kek = hkdf(&shared, b"glyph-pair", "glyph-pair-v1");
    let mut v = seal(&kek, format!("glyph-pair-v1\0{}", k.kid()).as_bytes(), &k.0);
    v["for"] = json!(pair_tag(their_msg));
    v["pake"] = json!(b64(&msg));
    v["kid"] = json!(k.kid());
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: &str, key: &str, v: Value) -> SyncItem {
        SyncItem { kind: kind.into(), key: key.into(), value: Some(v), updated_at: 7, deleted: false, rev: 3 }
    }

    #[test]
    fn items_round_trip_and_hide_their_key() {
        let k = DataKey::generate();
        let plain = item("liked", "audius:Abc12", json!({ "title": "Song", "artist": "Band" }));
        let sealed = k.seal_item(&plain);
        assert_eq!(sealed.kind, kind::E2E);
        assert!(!sealed.key.contains("audius"));
        let text = serde_json::to_string(&sealed).unwrap();
        assert!(!text.contains("Song") && !text.contains("Abc12"));
        assert_eq!(k.open_item(&sealed, |_| None).unwrap(), Some(plain));
        let pl = k.seal_item(&item("playlist", "p1", json!({ "name": "Mix", "tracks": [] })));
        assert_eq!(pl.kind, kind::E2E_PLAYLIST);
        assert_eq!(pl.rev, 3);
    }

    #[test]
    fn changed_or_moved_items_do_not_open() {
        let k = DataKey::generate();
        let a = k.seal_item(&item("liked", "a", json!(1)));
        let b = k.seal_item(&item("liked", "b", json!(2)));
        let mut moved = a.clone();
        moved.key = b.key.clone();
        assert_eq!(k.open_item(&moved, |_| None), Err(OpenError::Tampered));
        let mut changed = a.clone();
        let ct = changed.value.as_ref().unwrap()["ct"].as_str().unwrap().to_string();
        let mut bytes = unb64(&ct).unwrap();
        bytes[0] ^= 1;
        changed.value.as_mut().unwrap()["ct"] = json!(b64(&bytes));
        assert_eq!(k.open_item(&changed, |_| None), Err(OpenError::Tampered));
        let other = DataKey::generate();
        assert!(matches!(other.open_item(&a, |_| None), Err(OpenError::OtherKey(_))));
    }

    #[test]
    fn tombstones_need_the_plain_key() {
        let k = DataKey::generate();
        let gone = SyncItem { kind: "liked".into(), key: "a".into(), value: None, updated_at: 9, deleted: true, rev: 0 };
        let sealed = k.seal_item(&gone);
        assert!(sealed.deleted && sealed.value.is_none());
        let blinded = sealed.key.clone();
        let opened = k.open_item(&sealed, |h| (h == blinded).then(|| ("liked".to_string(), "a".to_string()))).unwrap();
        assert_eq!(opened, Some(gone));
        assert_eq!(k.open_item(&sealed, |_| None).unwrap(), None);
    }

    #[test]
    fn padding_hides_small_differences() {
        let k = DataKey::generate();
        let a = k.seal_item(&item("search", "query:a", json!("a")));
        let b = k.seal_item(&item("search", "query:abcdefgh", json!("abcdefghijk")));
        assert_eq!(a.value.unwrap()["ct"].as_str().unwrap().len(), b.value.unwrap()["ct"].as_str().unwrap().len());
    }

    #[test]
    fn password_key_is_not_the_sign_in_key_for_any_salt() {
        let id = "acc1";
        let pw = "correct horse battery";
        let vault = password_key(pw, id).unwrap();
        // The attack: a server hands out the vault salt (or the old settings
        // salt) as sign-in salt and receives Argon2id(password, that salt).
        for prefix in [vault_kdf::SALT_PREFIX, "glyph-settings-v1:"] {
            let salt = Sha256::digest(format!("{prefix}{id}").as_bytes());
            assert!(suspicious_sign_in_salt(&salt[..16], id));
            let sign_in = crate::account::derive_key(pw, &salt[..16]).unwrap();
            assert_ne!(sign_in.as_slice(), vault.as_slice());
        }
        assert!(!suspicious_sign_in_salt(&[7u8; 16], id));
        let k = DataKey::generate();
        let w = wrap_with_password(&vault, id, &k);
        assert_eq!(unwrap_with_password(&vault, id, &k.kid(), &w), Some(k.clone()));
        assert_eq!(unwrap_with_password(&password_key("wrong password!", id).unwrap(), id, &k.kid(), &w), None);
        assert_eq!(unwrap_with_password(&vault, "other", &k.kid(), &w), None);
    }

    #[test]
    fn device_envelopes_open_only_for_their_device() {
        let k = DataKey::generate();
        let (d1, d2) = (DeviceKey::generate(), DeviceKey::generate());
        let env = seal_to_device("dev1", &d1.public_b64(), &k, "dev0", None).unwrap();
        assert_eq!(open_from_device("dev1", &d1, &env), Some(k.clone()));
        assert_eq!(open_from_device("dev1", &d2, &env), None);
        assert_eq!(open_from_device("dev2", &d1, &env), None);
        assert_eq!(DeviceKey::from_b64(&d1.to_b64()).unwrap().public(), d1.public());
    }

    #[test]
    fn rotations_prove_the_previous_key() {
        let (old, new) = (DataKey::generate(), DataKey::generate());
        let d = DeviceKey::generate();
        let env = seal_to_device("dev1", &d.public_b64(), &new, "dev0", Some(&old)).unwrap();
        assert!(rotation_proven(&env, "dev1", &old));
        assert!(!rotation_proven(&env, "dev1", &DataKey::generate()));
        let plain = seal_to_device("dev1", &d.public_b64(), &new, "dev0", None).unwrap();
        assert!(!rotation_proven(&plain, "dev1", &old));
    }

    #[test]
    fn pairing_hands_over_the_key_only_with_the_secret() {
        let k = DataKey::generate();
        let start = PairStart::new();
        let answer = pair_answer(&start.secret, &start.msg, &k).unwrap();
        assert_eq!(start.finish(&answer).unwrap(), Some(k.clone()));
        // Someone in the middle without the secret part.
        let start = PairStart::with_secret("ABCD");
        let wrong = pair_answer("ABCE", &start.msg, &k).unwrap();
        assert_eq!(start.finish(&wrong), Err(OpenError::Tampered));
        // An entry for another pairing is not ours.
        let start = PairStart::new();
        let other = PairStart::new();
        let answer = pair_answer(&other.secret, &other.msg, &k).unwrap();
        assert_eq!(start.finish(&answer).unwrap(), None);
    }

    #[test]
    fn pairing_codes_split_into_server_and_secret_part() {
        assert_eq!(split_pair_code("k7px-2qmd"), Some(("K7PX-2QMD".into(), None)));
        assert_eq!(split_pair_code("K7PX-2QMD-H4XA"), Some(("K7PX-2QMD".into(), Some("H4XA".into()))));
        assert_eq!(split_pair_code("K7PX-2QM"), None);
        assert_eq!(split_pair_code("K7PX-2QMD-H4X0"), None);
    }
}
