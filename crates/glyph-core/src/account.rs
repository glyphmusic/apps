//! The signed-in account of this device (optional, see `docs/ACCOUNTS.md`).
//!
//! `account.json` in the data dir holds who is signed in where (no secrets).
//! The refresh token sits in the OS keychain (file fallback where there is
//! none); the short-lived access token only in memory. A session belongs to
//! one server URL; switching servers leaves it dormant.

use crate::config::Config;
use crate::server_client::{server_error, ServerClient};
use crate::settings_sync::{self, SettingsApplied, Vault};
use crate::sync::{self, Applied, Key, Snapshot, SyncState};
use crate::{paths, Backend, Library};
use anyhow::{anyhow, Context, Result};
use glyph_protocol::{
    password_kdf, Account, AccountPatch, ApproveResponse, DevicesResponse, HandleCheck, LinkStartResponse,
    LoginPollResponse, LoginStartResponse, PasswordLoginRequest, PasswordRegisterRequest, PasswordSetRequest,
    ProfilePatch, ProfilePlaylistSongs, ProfileSettings, PublicProfile, Session, SyncItem, Track,
};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// What `account.json` holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stored {
    pub server: String,
    pub account: Account,
    pub device_id: String,
}

struct Live {
    stored: Stored,
    access: Option<(String, Instant)>,
}

fn session() -> &'static Mutex<Option<Live>> {
    static S: OnceLock<Mutex<Option<Live>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(load_stored().map(|stored| Live { stored, access: None })))
}

#[cfg(test)]
pub(crate) fn with_test_access<T>(base: &str, token: &str, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<Live>);
    impl Drop for Restore {
        fn drop(&mut self) { *session().lock().unwrap() = self.0.take(); }
    }
    let stored = Stored { server: base.into(), device_id: "header-test".into(),
        account: Account { id: "header-test".into(), handle: "header_test".into(), device_limit: 5, created_at: 0, has_password: false } };
    let _restore = Restore(session().lock().unwrap().replace(Live {
        stored, access: Some((token.into(), Instant::now() + Duration::from_secs(60))),
    }));
    f()
}

fn sync_state() -> &'static Mutex<Option<SyncState>> {
    static S: OnceLock<Mutex<Option<SyncState>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

fn require_session(expected: &Stored) -> Result<()> {
    let guard = session().lock().unwrap();
    if guard.as_ref().is_some_and(|s| &s.stored == expected) { Ok(()) }
    else { Err(anyhow!(SIGNED_OUT)) }
}

fn stored_path() -> PathBuf {
    paths::data_dir().join("account.json")
}

fn load_stored() -> Option<Stored> {
    serde_json::from_slice(&std::fs::read(stored_path()).ok()?).ok()
}

/// A private cache for one account on one server, independent of the guest library.
pub fn library_path_for(acc: &Stored) -> PathBuf {
    use sha2::{Digest, Sha256};
    let scope = format!("{}\n{}", acc.server.trim_end_matches('/'), acc.account.id);
    let key: String = Sha256::digest(scope.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    paths::data_dir().join("accounts").join(key).join("library.json")
}

pub fn library_path() -> Option<PathBuf> {
    let acc = load_stored()?;
    let cfg = Config::load().ok()?;
    if cfg.effective_server_url().trim_end_matches('/') != acc.server.trim_end_matches('/') { return None; }
    Some(library_path_for(&acc))
}

pub fn guest_merge_marker() -> PathBuf { paths::data_dir().join("guest-merge-pending.json") }

pub fn guest_merge_pending(server: &str) -> bool {
    paths::read_json::<serde_json::Value>(&guest_merge_marker())
        .and_then(|v| v["server"].as_str().map(str::to_owned))
        .is_some_and(|s| s.trim_end_matches('/') == server.trim_end_matches('/'))
}

pub fn stored_guest_merge_pending() -> bool {
    load_stored().is_some_and(|s| guest_merge_pending(&s.server))
}

// ------------------------------------------------------------------ secrets

#[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "linux"))]
const KEYCHAIN_SERVICE: &str = "glyph";

/// Secure storage of the host app (Android Keystore, iOS Keychain). When set,
/// it is used before the OS keychain and the file fallback. The desktop app
/// never sets it.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
    /// `false` when the value could not be stored (the file fallback is used then).
    fn set(&self, key: &str, value: &str) -> bool;
    fn delete(&self, key: &str);
}

static SECRET_STORE: std::sync::RwLock<Option<std::sync::Arc<dyn SecretStore>>> = std::sync::RwLock::new(None);

pub fn set_secret_store(store: std::sync::Arc<dyn SecretStore>) {
    *SECRET_STORE.write().unwrap() = Some(store);
}

pub(crate) fn secret_store() -> Option<std::sync::Arc<dyn SecretStore>> {
    SECRET_STORE.read().unwrap().clone()
}

fn fallback_path(device_id: &str) -> PathBuf {
    paths::data_dir().join(format!("account_token_{device_id}"))
}

#[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "linux"))]
fn entry(device_id: &str) -> Option<keyring::Entry> {
    keyring::Entry::new(KEYCHAIN_SERVICE, &format!("account:{device_id}")).ok()
}

fn put_refresh(device_id: &str, token: &str) {
    if secret_store().is_some_and(|s| s.set(&format!("account:{device_id}"), token)) {
        let _ = std::fs::remove_file(fallback_path(device_id));
        return;
    }
    #[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "linux"))]
    if let Some(e) = entry(device_id) {
        if e.set_password(token).is_ok() {
            let _ = std::fs::remove_file(fallback_path(device_id));
            return;
        }
    }
    tracing::warn!("no OS keychain, keeping the account token in the data folder");
    if let Err(e) = paths::write_atomic(&fallback_path(device_id), token.as_bytes()) {
        tracing::warn!("could not store account token: {e}");
    }
}

fn get_refresh(device_id: &str) -> Option<String> {
    if let Some(t) = secret_store().and_then(|s| s.get(&format!("account:{device_id}"))) {
        return Some(t);
    }
    #[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "linux"))]
    if let Some(t) = entry(device_id).and_then(|e| e.get_password().ok()) {
        return Some(t);
    }
    std::fs::read_to_string(fallback_path(device_id)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn forget_refresh(device_id: &str) {
    if let Some(s) = secret_store() {
        s.delete(&format!("account:{device_id}"));
    }
    #[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "linux"))]
    if let Some(e) = entry(device_id) {
        let _ = e.delete_credential();
    }
    let _ = std::fs::remove_file(fallback_path(device_id));
}

// Keys of an account on this device: keychain entry per account, file
// fallback. `settings-key` is the old password-derived key of the synced API
// keys (still opens their old copies); the others belong to `e2e`.

fn put_secret(name: &str, fallback: &str, text: &str) {
    let path = paths::data_dir().join(fallback);
    if secret_store().is_some_and(|s| s.set(name, text)) {
        let _ = std::fs::remove_file(&path);
        return;
    }
    #[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "linux"))]
    if let Ok(e) = keyring::Entry::new(KEYCHAIN_SERVICE, name) {
        if e.set_password(text).is_ok() {
            let _ = std::fs::remove_file(&path);
            return;
        }
    }
    if let Err(e) = paths::write_atomic(&path, text.as_bytes()) {
        tracing::warn!("could not store {fallback}: {e}");
    }
}

fn get_secret(name: &str, fallback: &str) -> Option<String> {
    if let Some(t) = secret_store().and_then(|s| s.get(name)) {
        return Some(t);
    }
    #[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "linux"))]
    if let Some(t) = keyring::Entry::new(KEYCHAIN_SERVICE, name).ok().and_then(|e| e.get_password().ok()) {
        return Some(t);
    }
    std::fs::read_to_string(paths::data_dir().join(fallback)).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

fn forget_secret(name: &str, fallback: &str) {
    if let Some(s) = secret_store() {
        s.delete(name);
    }
    #[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "linux"))]
    if let Ok(e) = keyring::Entry::new(KEYCHAIN_SERVICE, name) {
        let _ = e.delete_credential();
    }
    let _ = std::fs::remove_file(paths::data_dir().join(fallback));
}

fn get_vault(account_id: &str) -> Option<Vault> {
    get_secret(&format!("settings-key:{account_id}"), &format!("settings_key_{account_id}")).and_then(|t| unb64(&t)).and_then(|b| Vault::from_bytes(&b))
}

fn forget_vault(account_id: &str) {
    forget_secret(&format!("settings-key:{account_id}"), &format!("settings_key_{account_id}"));
}

/// The account's data keys on this device: the current one first, then
/// older ones (they still open items from before a key change).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KeyBox {
    pub keys: Vec<crate::e2e::DataKey>,
}

impl KeyBox {
    pub fn current(&self) -> Option<&crate::e2e::DataKey> {
        self.keys.first()
    }

    /// Makes `k` current; the old current key stays for reading.
    pub fn adopt(&mut self, k: crate::e2e::DataKey) {
        self.keys.retain(|x| *x != k);
        self.keys.insert(0, k);
        self.keys.truncate(4);
    }
}

fn load_keys(account_id: &str) -> KeyBox {
    let keys = get_secret(&format!("data-key:{account_id}"), &format!("data_key_{account_id}"))
        .map(|t| t.split(',').filter_map(crate::e2e::DataKey::from_b64).collect())
        .unwrap_or_default();
    KeyBox { keys }
}

fn save_keys(account_id: &str, b: &KeyBox) {
    let text: Vec<String> = b.keys.iter().map(|k| k.to_b64()).collect();
    put_secret(&format!("data-key:{account_id}"), &format!("data_key_{account_id}"), &text.join(","));
}

fn device_key(account_id: &str) -> crate::e2e::DeviceKey {
    let (name, file) = (format!("device-key:{account_id}"), format!("device_key_{account_id}"));
    if let Some(k) = get_secret(&name, &file).and_then(|t| crate::e2e::DeviceKey::from_b64(&t)) {
        return k;
    }
    let k = crate::e2e::DeviceKey::generate();
    put_secret(&name, &file, &k.to_b64());
    k
}

/// The password key; `fresh` after a password change here, until the data
/// key is wrapped with it (kept with the key, so no sync round can lose it).
fn put_pw_key(account_id: &str, key: &[u8; 32], fresh: bool) {
    let text = if fresh { format!("new:{}", b64(key)) } else { b64(key) };
    put_secret(&format!("vault-key:{account_id}"), &format!("vault_key_{account_id}"), &text);
}

fn get_pw_key(account_id: &str) -> Option<([u8; 32], bool)> {
    let t = get_secret(&format!("vault-key:{account_id}"), &format!("vault_key_{account_id}"))?;
    let (fresh, t) = match t.strip_prefix("new:") {
        Some(rest) => (true, rest.to_string()),
        None => (false, t),
    };
    unb64(&t).and_then(|b| b.try_into().ok()).map(|k| (k, fresh))
}

/// Forget every key of an account on this device (sign-out, deletion).
fn forget_account_keys(account_id: &str) {
    forget_vault(account_id);
    forget_secret(&format!("data-key:{account_id}"), &format!("data_key_{account_id}"));
    forget_secret(&format!("device-key:{account_id}"), &format!("device_key_{account_id}"));
    forget_secret(&format!("vault-key:{account_id}"), &format!("vault_key_{account_id}"));
}

/// The new device's half of a pairing that is waiting for the key.
struct PendingPair {
    server_code: String,
    start: crate::e2e::PairStart,
    since: Instant,
}

fn pending_pair() -> &'static Mutex<Option<PendingPair>> {
    static P: OnceLock<Mutex<Option<PendingPair>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(None))
}

/// How long a paired device waits for the approving device's key.
const PAIR_WAIT: Duration = Duration::from_secs(15 * 60);

/// The key that wraps the data key with the password ([`crate::e2e::password_key`]).
async fn derive_pw_key(password: String, account_id: String) -> Result<[u8; 32]> {
    tokio::task::spawn_blocking(move || crate::e2e::password_key(&password, &account_id)).await.map_err(|e| anyhow!("key derivation: {e}"))?
}

/// A sign-in salt from the server, checked: the right length, and after
/// sign-in (`account_id`) not one that would reveal a data key.
fn checked_salt(raw: &str) -> Result<Vec<u8>> {
    unb64(raw).filter(|s| s.len() == password_kdf::SALT_LEN).ok_or_else(|| anyhow!("bad_salt"))
}

fn check_salt_after_sign_in(server: &str, salt: &[u8], account: &Account) {
    if crate::e2e::suspicious_sign_in_salt(salt, &account.id) {
        crate::privacy::record(server, &account.id, true, "salt_attack", "");
    }
}

/// Store a fresh session for `server` and make it current.
fn adopt(server: &str, s: &Session) {
    let _ = adopt_if_current(server, s, None);
}

fn adopt_if_current(server: &str, s: &Session, expected: Option<&Stored>) -> Result<()> {
    let mut current = session().lock().unwrap();
    if expected.is_some_and(|expected| current.as_ref().is_none_or(|live| live.stored != *expected)) {
        return Err(anyhow!(SIGNED_OUT));
    }
    let stored = Stored { server: server.to_string(), account: s.account.clone(), device_id: s.device_id.clone() };
    put_refresh(&s.device_id, &s.refresh_token);
    if let Err(e) = paths::write_json(&stored_path(), &stored) {
        tracing::warn!("could not save account: {e}");
    }
    let exp = Instant::now() + Duration::from_secs(s.access_expires_in_s.saturating_sub(60));
    *current = Some(Live { stored, access: Some((s.access_token.clone(), exp)) });
    Ok(())
}

/// Forget the local session (tokens, state). The library stays.
fn drop_local() {
    drop_local_if_current(None);
}

fn drop_local_if_current(expected: Option<&Stored>) {
    let mut current = session().lock().unwrap();
    if expected.is_some_and(|expected| current.as_ref().is_none_or(|live| live.stored != *expected)) { return; }
    if let Some(live) = current.take() {
        forget_refresh(&live.stored.device_id);
        forget_account_keys(&live.stored.account.id);
    }
    let _ = std::fs::remove_file(stored_path());
    // Keep the account's sync agreement with its private library cache.
    *sync_state().lock().unwrap() = None;
}

/// `Argon2id(password, salt)` with the shared parameters. The password itself
/// never leaves the device; the server only sees (and hashes) this key.
pub fn derive_key(password: &str, salt: &[u8]) -> Result<Vec<u8>> {
    use argon2::{Algorithm, Argon2, Params, Version};
    let params = Params::new(password_kdf::M_KIB, password_kdf::T_COST, password_kdf::P_COST, Some(password_kdf::KEY_LEN))
        .map_err(|e| anyhow!("argon2 params: {e}"))?;
    let mut out = vec![0u8; password_kdf::KEY_LEN];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, &mut out)
        .map_err(|e| anyhow!("argon2: {e}"))?;
    Ok(out)
}

/// Why a password cannot be used (checked here: the server never sees it).
pub fn password_problem(pw: &str) -> Option<&'static str> {
    if pw.chars().count() < password_kdf::MIN_PASSWORD_CHARS {
        Some("too_short")
    } else if pw.len() > 256 {
        Some("too_long")
    } else {
        None
    }
}

fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn unb64(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok()
}

async fn derive_async(password: String, salt: Vec<u8>) -> Result<Vec<u8>> {
    tokio::task::spawn_blocking(move || derive_key(&password, &salt)).await.map_err(|e| anyhow!("key derivation: {e}"))?
}

/// Platform name as the server expects it.
pub fn platform() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "android") {
        "android"
    } else if cfg!(target_os = "ios") {
        "ios"
    } else {
        "other"
    }
}

/// A readable default name for this device.
pub fn device_name() -> String {
    let host = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_default();
    let os = match platform() {
        "windows" => "Windows",
        "macos" => "macOS",
        "linux" => "Linux",
        "android" => "Android",
        "ios" => "iOS",
        _ => "Desktop",
    };
    if host.trim().is_empty() { format!("Glyph ({os})") } else { format!("{} ({os})", host.trim()) }
}

/// Results of one sync round's network part, applied with [`Backend::sync_apply`].
#[derive(Debug, Default)]
pub struct SyncRound {
    pub owner: Option<Stored>,
    pub bootstrap: bool,
    /// Remote items to apply, in rev order.
    pub items: Vec<SyncItem>,
    pub pushed: usize,
    /// Waiting for something another device sends soon (a pairing's key, a
    /// keyring written at the same time): sync again in a few seconds.
    pub retry_soon: bool,
}

/// Bumped when this version must read the whole account once.
const E2E_VERSION: u32 = 1;
/// How often the profile's playlists are read (they stay readable).
const PUBLIC_EVERY_MS: u64 = 60 * 60 * 1000;

fn is_official(base: &str) -> bool {
    base.trim_end_matches('/').eq_ignore_ascii_case(crate::config::DEFAULT_SERVER_URL)
}

enum PushMode {
    /// The keyring's key is here: items go out encrypted.
    Encrypt,
    /// The account is encrypted but this device lacks the current key (or
    /// its keyring is not on the server yet): nothing leaves.
    Hold,
    /// No keyring yet: as before, until this round creates one.
    Plain,
}

fn push_mode(st: &SyncState, keys: &KeyBox) -> PushMode {
    match (keys.current(), st.e2e.main()) {
        (Some(k), Some(m)) if m.kid == k.kid() => PushMode::Encrypt,
        (None, None) => PushMode::Plain,
        _ => PushMode::Hold,
    }
}

/// A new data key: every item goes up again, encrypted with it.
fn reseal_all(st: &mut SyncState) {
    let known: Vec<String> = st.known.iter().filter(|(_, h)| h.as_str() != "-").map(|(j, _)| j.clone()).collect();
    let sealed: Vec<String> = st.e2e.sealed.iter().cloned().collect();
    st.e2e.reseal.extend(known.into_iter().chain(sealed));
}

/// Brings this device's data key in line with the account's keyring (see
/// `e2e`): takes a key that reached it, creates the keyring, keeps the
/// password copy and this device's entry current, and on the official server
/// hands the key to new devices. Keyring writes go out right away.
async fn settle_keys(c: &ServerClient, acc: &Stored, st: &mut SyncState, keys: &mut KeyBox, official: bool, now: u64, round: &mut SyncRound) -> Result<()> {
    use crate::e2e::{self, DataKey};
    use crate::e2e_sync::{keyring_item, Choice, Chosen, Main, Source};
    let id = acc.account.id.as_str();
    let me = acc.device_id.as_str();
    let server = c.base_url().to_string();
    let owned = id.to_string();
    let (dk, pw) = tokio::task::spawn_blocking(move || (device_key(&owned), get_pw_key(&owned))).await?;
    let pw_fresh = pw.is_some_and(|(_, fresh)| fresh);
    let pw_key = pw.map(|(k, _)| k);
    let before = keys.clone();
    let mut writes: Vec<SyncItem> = Vec::new();

    // A paired device: the approving device's entry with the key.
    {
        let mut g = pending_pair().lock().unwrap();
        if g.as_ref().is_some_and(|p| p.since.elapsed() > PAIR_WAIT) {
            *g = None;
        }
        let current = st.e2e.main().is_some_and(|m| keys.current().is_some_and(|k| k.kid() == m.kid));
        let entry = g.as_ref().and_then(|p| {
            let key = format!("{}{}", e2e::PAIR, p.server_code);
            st.e2e.keyring.get(&key).filter(|v| p.start.is_for(v)).cloned().map(|v| (key, v))
        });
        match (entry, current) {
            (Some((key, v)), false) => {
                let p = g.take().expect("checked above");
                match p.start.finish(&v) {
                    Ok(Some(k)) => keys.adopt(k),
                    Ok(None) => {}
                    Err(_) => crate::privacy::record(&server, id, true, "pair_tampered", ""),
                }
                writes.push(keyring_item(&key, None, now));
            }
            (Some((key, _)), true) => {
                *g = None;
                writes.push(keyring_item(&key, None, now));
            }
            (None, false) if g.is_some() => round.retry_soon = true,
            _ => {}
        }
    }

    let choice = Choice { account_id: id, device_id: me, current: keys.current(), pw_key: pw_key.as_ref(), device: &dk, official };
    match crate::e2e_sync::choose(&st.e2e, &choice) {
        Chosen::Keep | Chosen::Locked => {}
        Chosen::Take(k, source) => {
            if source == Source::Escrow {
                st.e2e.escrowed.insert(k.kid());
            }
            let had = keys.current().is_some();
            keys.adopt(k);
            if had {
                reseal_all(st);
            }
        }
        Chosen::Refused => {
            crate::privacy::record(&server, id, true, "keyring_replaced", "");
            if let Some(k) = keys.current() {
                let pw = pw_key.map(|p| e2e::wrap_with_password(&p, id, k));
                let escrow = st.e2e.escrowed.contains(&k.kid());
                writes.push(keyring_item(e2e::MAIN, Some(Main::value(k, pw, escrow)), now));
            }
        }
        Chosen::NoKeyring => {
            if keys.current().is_some() && st.e2e.keyring_seen {
                crate::privacy::record(&server, id, true, "keyring_removed", "");
            }
            let fresh = keys.current().is_none();
            let k = keys.current().cloned().unwrap_or_else(DataKey::generate);
            // Without the password here the server keeps the key, so no
            // library can be lost; the first device with the password
            // replaces it with a key the server never sees.
            let escrow = pw_key.is_none() || st.e2e.escrowed.contains(&k.kid());
            if escrow {
                st.e2e.escrowed.insert(k.kid());
            }
            let pw = pw_key.map(|p| e2e::wrap_with_password(&p, id, &k));
            writes.push(keyring_item(e2e::MAIN, Some(Main::value(&k, pw, escrow)), now));
            if fresh {
                keys.adopt(k);
                // Everything on the server so far is readable: replace it.
                let known: Vec<String> = st.known.iter().filter(|(_, h)| h.as_str() != "-").map(|(j, _)| j.clone()).collect();
                st.e2e.mark_plain(known);
            }
        }
    }

    let main = writes.iter().rev().find(|w| w.key == e2e::MAIN).and_then(|w| w.value.as_ref()).and_then(Main::parse).or_else(|| st.e2e.main());
    match (keys.current().cloned(), main) {
        (Some(k), Some(main)) if main.kid == k.kid() => {
            let mut k = k;
            let pw_key = match pw_key {
                // The password changed on another device: this key is stale
                // and must never overwrite the current copy.
                Some(pw) if !pw_fresh && main.pw.as_ref().is_some_and(|v| e2e::unwrap_with_password(&pw, id, &main.kid, v).is_none()) => {
                    let owned = id.to_string();
                    tokio::task::spawn_blocking(move || forget_secret(&format!("vault-key:{owned}"), &format!("vault_key_{owned}"))).await?;
                    None
                }
                p => p,
            };
            if let Some(pw) = pw_key {
                let server_knew = main.escrow.is_some() || st.e2e.escrowed.contains(&k.kid());
                let pw_ok = !pw_fresh && main.pw.is_some();
                if server_knew {
                    // The password is here: a new key the server never sees.
                    let k2 = DataKey::generate();
                    writes.retain(|w| w.key != e2e::MAIN);
                    writes.push(keyring_item(e2e::MAIN, Some(Main::value(&k2, Some(e2e::wrap_with_password(&pw, id, &k2)), false)), now));
                    for (key, v) in &st.e2e.keyring {
                        let (Some(dev), Some(pk)) = (key.strip_prefix(e2e::DEV), v["pk"].as_str()) else { continue };
                        if dev == me {
                            continue;
                        }
                        if let Some(env) = e2e::seal_to_device(dev, pk, &k2, me, Some(&k)) {
                            writes.push(keyring_item(&format!("{}{dev}", e2e::ENV), Some(env), now));
                        }
                    }
                    keys.adopt(k2.clone());
                    k = k2;
                    reseal_all(st);
                    crate::privacy::record(&server, id, false, "key_replaced", "");
                } else if !pw_ok {
                    writes.retain(|w| w.key != e2e::MAIN);
                    writes.push(keyring_item(e2e::MAIN, Some(Main::value(&k, Some(e2e::wrap_with_password(&pw, id, &k)), false)), now));
                }
            }
            // This device's public key, proven with the data key.
            let mine = format!("{}{me}", e2e::DEV);
            let entry = serde_json::json!({ "pk": dk.public_b64(), "kid": k.kid(), "mac": k.device_mac(me, &dk.public()) });
            if st.e2e.keyring.get(&mine) != Some(&entry) {
                writes.push(keyring_item(&mine, Some(entry), now));
            }
            // The official server (trusted with sign-in anyway): new devices
            // of the account get the key without anything to type.
            if official {
                let mut names: Option<Vec<glyph_protocol::Device>> = None;
                let entries: Vec<(String, serde_json::Value)> = st.e2e.keyring.iter().map(|(a, b)| (a.clone(), b.clone())).collect();
                for (key, v) in entries {
                    let (Some(dev), Some(pk)) = (key.strip_prefix(e2e::DEV), v["pk"].as_str()) else { continue };
                    if dev == me {
                        continue;
                    }
                    let proven = unb64(pk).is_some_and(|pk| v["mac"].as_str() == Some(k.device_mac(dev, &pk).as_str()) && v["kid"].as_str() == Some(k.kid().as_str()));
                    let env_key = format!("{}{dev}", e2e::ENV);
                    let has_env = st.e2e.keyring.get(&env_key).is_some_and(|e| e["kid"].as_str() == Some(k.kid().as_str())) || writes.iter().any(|w| w.key == env_key);
                    if proven || has_env {
                        continue;
                    }
                    let Some(env) = e2e::seal_to_device(dev, pk, &k, me, None) else { continue };
                    writes.push(keyring_item(&env_key, Some(env), now));
                    if names.is_none() {
                        names = Some(with_auth(c, |t| {
                            let c = c.clone();
                            async move { c.devices(&t).await }
                        }).await.map(|d| d.devices).unwrap_or_default());
                    }
                    let name = names.iter().flatten().find(|d| d.id == dev).map(|d| d.name.clone()).unwrap_or_else(|| dev.chars().take(8).collect());
                    crate::privacy::record(&server, id, false, "key_shared", &name);
                }
            }
        }
        _ => {
            // No key here yet: publish this device's public key, so a device
            // with the key can seal it to this one.
            let mine = format!("{}{me}", e2e::DEV);
            if st.e2e.keyring.get(&mine).and_then(|v| v["pk"].as_str()) != Some(dk.public_b64().as_str()) {
                writes.push(keyring_item(&mine, Some(serde_json::json!({ "pk": dk.public_b64() })), now));
            }
        }
    }

    if !writes.is_empty() {
        let rewrapped = writes.iter().any(|w| w.key == e2e::MAIN && w.value.as_ref().is_some_and(|v| v.get("pw").is_some()));
        let r = with_auth(c, |t| {
            let (c, items, acc) = (c.clone(), writes.clone(), acc.clone());
            async move { require_session(&acc)?; c.sync_push(&t, items).await }
        })
        .await?;
        let stale: std::collections::BTreeSet<String> = r.stale.iter().filter(|i| i.kind == glyph_protocol::sync_kind::KEYRING).map(|i| i.key.clone()).collect();
        if let (true, false, Some(pw)) = (rewrapped && pw_fresh, stale.contains(e2e::MAIN), pw_key) {
            let owned = id.to_string();
            tokio::task::spawn_blocking(move || put_pw_key(&owned, &pw, false)).await?;
        }
        let done: Vec<SyncItem> = writes.into_iter().filter(|w| !stale.contains(&w.key)).collect();
        st.e2e.absorb_keyring(&done);
        st.e2e.absorb_keyring(&r.stale);
        if stale.contains(e2e::MAIN) {
            // Another device wrote the keyring at the same moment: decide again.
            *keys = before.clone();
            round.retry_soon = true;
        }
    }
    if *keys != before {
        let (owned, b) = (id.to_string(), keys.clone());
        tokio::task::spawn_blocking(move || save_keys(&owned, &b)).await?;
    }
    Ok(())
}

/// The approving device's half of a pairing: the data key for the device
/// that showed the code (see `e2e::pair_answer`).
async fn hand_over_key(c: &ServerClient, acc: &Stored, server_code: &str, secret: &str, their_msg: &str) -> Result<()> {
    let owned = acc.account.id.clone();
    let keys = tokio::task::spawn_blocking(move || load_keys(&owned)).await?;
    let k = keys.current().cloned().ok_or_else(|| anyhow!("no data key on this device"))?;
    let main_kid = {
        let mut g = sync_state().lock().unwrap();
        g.get_or_insert_with(|| SyncState::load(&acc.account.id)).e2e.main().map(|m| m.kid)
    };
    if main_kid.as_deref() != Some(k.kid().as_str()) {
        return Err(anyhow!("this device does not have the current data key"));
    }
    let v = crate::e2e::pair_answer(secret, their_msg, &k).ok_or_else(|| anyhow!("bad pairing message"))?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let item = crate::e2e_sync::keyring_item(&format!("{}{server_code}", crate::e2e::PAIR), Some(v), now);
    with_auth(c, |t| {
        let (c, item) = (c.clone(), item.clone());
        async move { c.sync_push(&t, vec![item]).await }
    })
    .await?;
    Ok(())
}

/// Is this an error that means "this device is no longer signed in"?
pub fn is_signed_out(e: &anyhow::Error) -> bool {
    e.to_string() == SIGNED_OUT
}

const SIGNED_OUT: &str = "signed out";

/// Only disclose account credentials to their bound HTTPS server (or loopback).
pub(crate) async fn optional_access(client: &ServerClient) -> Option<String> {
    if !crate::server_access::secure_base(client.base_url()) { return None; }
    // Metadata must not queue behind a slow account refresh. Account calls
    // refresh the session independently; anonymous metadata remains usable.
    cached_access(client.base_url())
}

pub(crate) fn cached_access(base: &str) -> Option<String> {
    if !crate::server_access::secure_base(base) { return None; }
    let g = session().lock().unwrap();
    let live = g.as_ref()?;
    if live.stored.server != base { return None; }
    live.access.as_ref().filter(|(_, exp)| *exp > Instant::now()).map(|(token, _)| token.clone())
}

/// Short-lived media credential; restricted to the signed-in server's URL.
pub fn media_access(url: &str) -> Option<String> {
    let base = session().lock().unwrap().as_ref()?.stored.server.clone();
    if !crate::server_access::same_server(&base, url) { return None; }
    cached_access(&base)
}

async fn access_token(client: &ServerClient, force: bool) -> Result<String> {
    // Concurrent metadata/cover requests must not race rotating refresh tokens.
    static REFRESH: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    if !force {
        if let Some(token) = cached_access(client.base_url()) { return Ok(token); }
    }
    let _refresh = tokio::time::timeout(Duration::from_secs(15), REFRESH.get_or_init(|| tokio::sync::Mutex::new(())).lock())
        .await.map_err(|_| anyhow!("Account refresh is taking too long. Please try again."))?;
    let (expected, cached) = {
        let g = session().lock().unwrap();
        let live = g.as_ref().ok_or_else(|| anyhow!(SIGNED_OUT))?;
        if live.stored.server != client.base_url() {
            return Err(anyhow!(SIGNED_OUT));
        }
        (live.stored.clone(), live.access.clone())
    };
    if let (false, Some((tok, exp))) = (force, cached) {
        if exp > Instant::now() {
            return Ok(tok);
        }
    }
    let device_id = expected.device_id.clone();
    let refresh = tokio::task::spawn_blocking(move || get_refresh(&device_id)).await?
        .ok_or_else(|| anyhow!(SIGNED_OUT))?;
    match client.refresh_session(&refresh).await {
        Ok(s) => {
            let (base, fresh) = (client.base_url().to_string(), s.clone());
            tokio::task::spawn_blocking(move || adopt_if_current(&base, &fresh, Some(&expected))).await??;
            Ok(s.access_token)
        }
        Err(e) if matches!(server_error(&e), Some((401, _))) => {
            drop_local_if_current(Some(&expected));
            Err(anyhow!(SIGNED_OUT))
        }
        Err(e) => Err(e),
    }
}

/// Run `f` with a valid access token; refresh once on expiry, sign out locally
/// when the server no longer knows this device.
async fn with_auth<T, F, Fut>(client: &ServerClient, f: F) -> Result<T>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let expected = session().lock().unwrap().as_ref().map(|s| s.stored.clone()).ok_or_else(|| anyhow!(SIGNED_OUT))?;
    let tok = access_token(client, false).await?;
    require_session(&expected)?;
    match f(tok).await {
        Err(e) if matches!(server_error(&e), Some((401, "token_expired"))) => {
            require_session(&expected)?;
            let token = access_token(client, true).await?;
            require_session(&expected)?;
            f(token).await
        }
        Err(e) if matches!(server_error(&e), Some((401, _))) => {
            drop_local_if_current(Some(&expected));
            Err(anyhow!(SIGNED_OUT))
        }
        r => r,
    }
}

impl Backend {
    pub async fn playback_live(&self) -> Result<crate::server_client::PlaybackLive> {
        let expected = self.account().ok_or_else(|| anyhow!(SIGNED_OUT))?;
        let c = self.account_client()?;
        self.spawn(async move { with_auth(&c, |token| {
            let (c, expected) = (c.clone(), expected.clone());
            async move { require_session(&expected)?; c.playback_live(&token).await }
        }).await }).await?
    }

    pub async fn playback_ack(&self, receipt: glyph_protocol::playback::Receipt) -> Result<()> {
        let expected = self.account().ok_or_else(|| anyhow!(SIGNED_OUT))?;
        let c = self.account_client()?;
        self.spawn(async move { with_auth(&c, |token| {
            let (c, receipt, expected) = (c.clone(), receipt.clone(), expected.clone());
            async move { require_session(&expected)?; c.playback_ack(&token, &receipt).await }
        }).await }).await?
    }
    pub fn playback_send(&self, mut send: glyph_protocol::playback::Send) -> std::sync::mpsc::Receiver<Result<()>> {
        send.request_id = Some(uuid::Uuid::new_v4().simple().to_string());
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let setup = self.account().ok_or_else(|| anyhow!(SIGNED_OUT)).and_then(|expected| Ok((expected, self.account_client()?)));
        self.rt.as_ref().expect("runtime alive").spawn(async move {
            let r = async {
                let (expected, c) = setup?;
                with_auth(&c, |token| {
                    let (c, send, expected) = (c.clone(), send.clone(), expected.clone());
                    async move { require_session(&expected)?; c.playback_command(&token, &send).await }
                }).await
            }.await;
            let _ = tx.send(r);
        });
        rx
    }
    pub async fn playback_poll(&self, state: glyph_protocol::playback::State) -> Result<glyph_protocol::playback::Poll> {
        let expected = self.account().ok_or_else(|| anyhow!(SIGNED_OUT))?;
        let c = self.account_client()?;
        self.spawn(async move {
            with_auth(&c, |token| {
                let (c, state, expected) = (c.clone(), state.clone(), expected.clone());
                async move {
                    require_session(&expected)?;
                    c.playback_poll(&token, &state).await
                }
            }).await
        }).await?
    }

    pub async fn playback_command(&self, mut send: glyph_protocol::playback::Send) -> Result<()> {
        send.request_id = Some(uuid::Uuid::new_v4().simple().to_string());
        let expected = self.account().ok_or_else(|| anyhow!(SIGNED_OUT))?;
        let c = self.account_client()?;
        self.spawn(async move {
            with_auth(&c, |token| {
                let (c, send, expected) = (c.clone(), send.clone(), expected.clone());
                async move {
                    require_session(&expected)?;
                    c.playback_command(&token, &send).await
                }
            }).await
        }).await?
    }
    pub fn reset_library_sync(&self) {
        if let Some(acc) = self.account() {
            *sync_state().lock().unwrap() = Some(SyncState::load(&acc.account.id));
        }
    }

    /// Complete an accepted guest import only after applying a fresh cloud pull.
    /// Keep the source snapshot until the merged account cache is durable.
    pub fn sync_merge_guest(&self, lib: &mut Library) -> Result<bool> {
        let acc = self.account().ok_or_else(|| anyhow!(SIGNED_OUT))?;
        let path = library_path_for(&acc);
        let pending = path.with_file_name("pending-guest.json");
        if !pending.exists() { return Ok(false); }
        let guest = Library::load_from(&pending)?;
        sync::merge_guest(lib, &guest);
        lib.save_to(&path)?;
        std::fs::remove_file(pending)?;
        Ok(true)
    }
    fn account_client(&self) -> Result<ServerClient> {
        self.cur().server.clone().ok_or_else(|| anyhow!("no server configured"))
    }

    /// Signed-in account on the current server.
    pub fn account(&self) -> Option<Stored> {
        let base = self.cur().server.as_ref()?.base_url().to_string();
        session().lock().unwrap().as_ref().map(|l| l.stored.clone()).filter(|s| s.server == base)
    }

    /// Text to share for a list: a short link when signed in on the public
    /// server, else (or when that fails) the `glyph:playlist:` code. A list
    /// opened from a link and not changed since shares that same link, so its
    /// author stays the original one. `kind` is "album" for an artist's album.
    pub async fn share_text(&self, name: &str, tracks: &[Track], kind: &str, origin: Option<String>) -> String {
        let code = crate::share::encode(name, tracks);
        let official = self.cur().config.effective_server_url().trim_end_matches('/').eq_ignore_ascii_case(crate::config::DEFAULT_SERVER_URL);
        if !official || self.account().is_none() {
            return code;
        }
        let Ok(c) = self.account_client() else { return code };
        let (code2, name, songs, kind) = (code.clone(), name.to_string(), tracks.len(), kind.to_string());
        let r = self
            .spawn(async move {
                if let Some(link) = origin.filter(|o| crate::share::link_id(o).is_some()) {
                    let id = crate::share::link_id(&link).unwrap_or_default();
                    if c.shared_playlist(&id).await.is_ok_and(|p| crate::share::same_songs(&p.code, &code2)) {
                        return Ok(link);
                    }
                }
                with_auth(&c, |t| {
                    let (c, code, name, kind) = (c.clone(), code2.clone(), name.clone(), kind.clone());
                    async move { c.share_playlist(&t, &code, &name, songs, &kind).await }
                })
                .await
            })
            .await;
        match r {
            Ok(Ok(link)) => link,
            Ok(Err(e)) | Err(e) => {
                tracing::info!("short playlist link failed, sharing the code: {e:#}");
                code
            }
        }
    }

    /// The playlist behind a short link (see [`crate::share::link_id`]).
    pub async fn resolve_share_link(&self, link: &str) -> Result<crate::server_client::SharedPlaylist> {
        let id = crate::share::link_id(link).ok_or_else(|| anyhow!("not a Glyph playlist link"))?;
        let inner = self.cur();
        // Public links must never resolve against a custom account server.
        let server = ServerClient::new(crate::config::DEFAULT_SERVER_URL, inner.http.clone(), crate::server_client::install_id());
        self.spawn(async move { server.shared_playlist(&id).await }).await?
    }

    /// The current server offers accounts (`/v1/health`).
    pub async fn accounts_available(&self) -> bool {
        let Ok(c) = self.account_client() else { return false };
        self.spawn(async move { c.health().await.map(|h| h.accounts || h.supports(glyph_protocol::capability::ACCOUNTS)).unwrap_or(false) }).await.unwrap_or(false)
    }

    /// Starts a sign-in (browser page or pairing code). Where the server
    /// relays pairing messages, the code gets a part only this device knows:
    /// the approving device uses it to hand over the library's key.
    pub async fn account_login_start(&self) -> Result<LoginStartResponse> {
        let c = self.account_client()?;
        self.spawn(async move {
            let pair = c.health().await.is_ok_and(|h| h.supports(glyph_protocol::capability::PAIR_KEYS)).then(crate::e2e::PairStart::new);
            let mut r = c.login_start(&device_name(), platform(), pair.as_ref().map(|p| p.msg.clone())).await?;
            if let Some(p) = pair {
                let server_code = r.user_code.clone();
                r.user_code = format!("{server_code}-{}", p.secret);
                *pending_pair().lock().unwrap() = Some(PendingPair { server_code, start: p, since: Instant::now() });
            }
            Ok(r)
        })
        .await?
    }

    /// One poll; on success the session is stored and the tokens are not returned.
    pub async fn account_login_poll(&self, login: &LoginStartResponse) -> Result<LoginPollResponse> {
        let c = self.account_client()?;
        let (id, secret) = (login.login_id.clone(), login.poll_secret.clone());
        self.spawn(async move {
            let r = c.login_poll(&id, &secret).await?;
            if let LoginPollResponse::Done { session } = &r {
                drop_local();
                adopt(c.base_url(), session);
                let mut redacted = session.clone();
                redacted.access_token.clear();
                redacted.refresh_token.clear();
                return Ok(LoginPollResponse::Done { session: redacted });
            }
            Ok(r)
        })
        .await?
    }

    /// New account with handle + password. Returns the one-time recovery code.
    pub async fn account_password_signup(&self, handle: &str, password: &str) -> Result<String> {
        if let Some(p) = password_problem(password) {
            return Err(anyhow!("password_{p}"));
        }
        let (c, handle, password) = (self.account_client()?, handle.to_string(), password.to_string());
        self.spawn(async move {
            let salt: Vec<u8> = (0..password_kdf::SALT_LEN).map(|_| rand::random::<u8>()).collect();
            let key = derive_async(password.clone(), salt.clone()).await?;
            let body = PasswordRegisterRequest { handle, salt: b64(&salt), key: b64(&key), device_name: device_name(), platform: platform().into() };
            let r = c.password_register(&body).await?;
            drop_local();
            adopt(c.base_url(), &r.session);
            let id = r.session.account.id.clone();
            put_pw_key(&id, &derive_pw_key(password, id.clone()).await?, false);
            Ok(r.recovery_code)
        })
        .await?
    }

    /// Sign in with handle + password.
    pub async fn account_password_login(&self, handle: &str, password: &str) -> Result<Account> {
        let (c, handle, password) = (self.account_client()?, handle.to_string(), password.to_string());
        self.spawn(async move {
            let salt = match c.password_salt(&handle).await {
                Ok(s) => checked_salt(&s.salt).inspect_err(|_| crate::privacy::record(c.base_url(), "", true, "bad_salt", ""))?,
                // Same answer as a wrong password: do not tell which part was wrong.
                Err(e) if matches!(server_error(&e), Some((404, _))) => return Err(anyhow!("login_failed")),
                Err(e) => return Err(e),
            };
            let key = derive_async(password.clone(), salt.clone()).await?;
            let body = PasswordLoginRequest { handle, key: b64(&key), device_name: device_name(), platform: platform().into() };
            let s = c.password_login(&body).await?;
            drop_local();
            adopt(c.base_url(), &s);
            check_salt_after_sign_in(c.base_url(), &salt, &s.account);
            put_pw_key(&s.account.id, &derive_pw_key(password, s.account.id.clone()).await?, false);
            Ok(s.account)
        })
        .await?
    }

    /// Set or change the password of the signed-in account.
    pub async fn account_set_password(&self, password: &str) -> Result<Account> {
        if let Some(p) = password_problem(password) {
            return Err(anyhow!("password_{p}"));
        }
        let (c, password) = (self.account_client()?, password.to_string());
        self.spawn(async move {
            let salt: Vec<u8> = (0..password_kdf::SALT_LEN).map(|_| rand::random::<u8>()).collect();
            let key = derive_async(password.clone(), salt.clone()).await?;
            let body = PasswordSetRequest { salt: b64(&salt), key: b64(&key) };
            let a = with_auth(&c, |t| {
                let (c, b) = (c.clone(), body.clone());
                async move { c.set_password(&t, &b).await }
            })
            .await?;
            if let Some(live) = session().lock().unwrap().as_mut() {
                live.stored.account = a.clone();
                let _ = paths::write_json(&stored_path(), &live.stored);
            }
            // The next sync wraps the library's key with the new password
            // (and replaces a key the server kept while there was none).
            put_pw_key(&a.id, &derive_pw_key(password, a.id.clone()).await?, true);
            Ok(a)
        })
        .await?
    }

    /// Browser page that adds a passkey to the signed-in account.
    pub async fn account_link_passkey(&self) -> Result<LinkStartResponse> {
        let c = self.account_client()?;
        self.spawn(async move {
            with_auth(&c, |t| {
                let c = c.clone();
                async move { c.passkey_link(&t).await }
            })
            .await
        })
        .await?
    }

    pub async fn account_refresh(&self) -> Result<Account> {
        let c = self.account_client()?;
        self.spawn(async move {
            let a = with_auth(&c, |t| {
                let c = c.clone();
                async move { c.account_get(&t).await }
            })
            .await?;
            if let Some(live) = session().lock().unwrap().as_mut() {
                live.stored.account = a.clone();
                let _ = paths::write_json(&stored_path(), &live.stored);
            }
            Ok(a)
        })
        .await?
    }

    pub async fn account_update(&self, patch: AccountPatch) -> Result<Account> {
        let c = self.account_client()?;
        self.spawn(async move {
            let a = with_auth(&c, |t| {
                let (c, p) = (c.clone(), patch.clone());
                async move { c.account_patch(&t, &p).await }
            })
            .await?;
            if let Some(live) = session().lock().unwrap().as_mut() {
                live.stored.account = a.clone();
                let _ = paths::write_json(&stored_path(), &live.stored);
            }
            Ok(a)
        })
        .await?
    }

    pub async fn account_handle_check(&self, handle: &str) -> Result<HandleCheck> {
        let (c, h) = (self.account_client()?, handle.to_string());
        self.spawn(async move { c.handle_check(&h).await }).await?
    }

    pub async fn account_devices(&self) -> Result<DevicesResponse> {
        let c = self.account_client()?;
        self.spawn(async move {
            with_auth(&c, |t| {
                let c = c.clone();
                async move { c.devices(&t).await }
            })
            .await
        })
        .await?
    }

    pub async fn account_remove_device(&self, id: &str) -> Result<()> {
        let (c, id) = (self.account_client()?, id.to_string());
        self.spawn(async move {
            with_auth(&c, |t| {
                let (c, id) = (c.clone(), id.clone());
                async move { c.remove_device(&t, &id).await }
            })
            .await
        })
        .await?
    }

    /// Approve a pairing code shown on a new device. With the code's secret
    /// part, this device also hands over the library's key (see `e2e`).
    pub async fn account_approve(&self, user_code: &str) -> Result<ApproveResponse> {
        let acc = self.account().ok_or_else(|| anyhow!(SIGNED_OUT))?;
        let (c, code) = (self.account_client()?, user_code.to_string());
        self.spawn(async move {
            let (server_code, secret) = crate::e2e::split_pair_code(&code).map_or((code.clone(), None), |(s, x)| (s, x));
            let r = with_auth(&c, |t| {
                let (c, code) = (c.clone(), server_code.clone());
                async move { c.approve_login(&t, &code).await }
            })
            .await?;
            if let (Some(secret), Some(msg)) = (secret, r.pake.clone()) {
                if let Err(e) = hand_over_key(&c, &acc, &server_code, &secret, &msg).await {
                    tracing::info!("pairing key handover skipped: {e:#}");
                }
            }
            Ok(r)
        })
        .await?
    }

    pub async fn account_new_recovery_code(&self) -> Result<String> {
        let c = self.account_client()?;
        self.spawn(async move {
            with_auth(&c, |t| {
                let c = c.clone();
                async move { c.new_recovery_code(&t).await }
            })
            .await
        })
        .await?
    }

    /// Sign this device out (server and local). The library stays on the device.
    pub async fn account_sign_out(&self) -> Result<()> {
        let Some(expected) = self.account() else { return Ok(()) };
        let c = self.account_client()?;
        self.spawn(async move {
            let r = with_auth(&c, |t| {
                let c = c.clone();
                async move { c.sign_out_device(&t).await }
            })
            .await;
            drop_local_if_current(Some(&expected));
            match r {
                Err(e) if !is_signed_out(&e) => tracing::warn!("server sign-out failed, signed out locally: {e:#}"),
                _ => {}
            }
            Ok(())
        })
        .await?
    }

    /// Delete the account and everything synced. Local data stays.
    pub async fn account_delete(&self, confirm_handle: &str) -> Result<()> {
        let expected = self.account().ok_or_else(|| anyhow!(SIGNED_OUT))?;
        let (c, h) = (self.account_client()?, confirm_handle.to_string());
        self.spawn(async move {
            with_auth(&c, |t| {
                let (c, h) = (c.clone(), h.clone());
                async move { c.account_delete(&t, &h).await }
            })
            .await?;
            drop_local_if_current(Some(&expected));
            Ok(())
        })
        .await?
    }

    /// Songs this device should keep offline (downloaded + requested by sync).
    pub fn sync_pending(&self, snap: &Snapshot) -> usize {
        let Some(acc) = self.account() else { return 0 };
        let state = SyncState::load(&acc.account.id);
        state.changes(snap, 0).len().max(state.outbox.len())
    }

    pub fn sync_wanted_downloads(&self) -> Vec<Track> {
        let downloaded = self.downloaded_tracks();
        let Some(acc) = self.account() else { return downloaded };
        let mut g = sync_state().lock().unwrap();
        let st = g.get_or_insert_with(|| SyncState::load(&acc.account.id));
        sync::settle_downloads(st, &downloaded);
        st.wanted_downloads(&downloaded)
    }

    /// Network half of a sync round: push local changes (and new playlist
    /// pictures), pull what other devices did, fetch their pictures.
    ///
    /// `settings` are [`settings_sync::items`]; they are only pushed once this
    /// device has pulled the account's settings, so a fresh device does not
    /// overwrite them with its defaults.
    ///
    /// Items go out end-to-end encrypted ([`crate::e2e_sync`]) once the
    /// account has a keyring; until this device has the key nothing leaves it.
    pub async fn sync_exchange(&self, snap: Snapshot, settings: Vec<(Key, serde_json::Value)>) -> Result<SyncRound> {
        let acc = self.account().ok_or_else(|| anyhow!(SIGNED_OUT))?;
        let c = self.account_client()?;
        self.spawn(async move {
            require_session(&acc)?;
            let official = is_official(c.base_url());
            let mut snap = snap;
            let mut st = {
                let mut g = sync_state().lock().unwrap();
                g.get_or_insert_with(|| SyncState::load(&acc.account.id)).clone()
            };
            if !settings.iter().any(|(k, _)| *k == settings_sync::secret_key()) {
                // No settings key here: never push or delete the keys.
                st.forget(&settings_sync::secret_key());
            }
            if st.pulled_once {
                snap.items.extend(settings);
            }
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
            let mut round = SyncRound::default();
            round.owner = Some(acc.clone());
            round.bootstrap = !st.pulled_once;
            // Read the whole account once with this version: older versions
            // skipped keyring entries they did not know.
            if st.e2e.version < E2E_VERSION {
                st.last_rev = 0;
                st.e2e.version = E2E_VERSION;
            }
            let owned = acc.account.id.clone();
            let mut keys = tokio::task::spawn_blocking(move || load_keys(&owned)).await?;
            let mut notices = Vec::new();

            // Pictures first, so other devices can fetch them once the playlist arrives.
            for (sha, path) in &snap.cover_files {
                if st.uploaded_covers.contains(sha) {
                    continue;
                }
                let Ok(bytes) = std::fs::read(path) else { continue };
                if bytes.len() > 512 * 1024 {
                    continue;
                }
                let (sha2, cc) = (sha.clone(), c.clone());
                match with_auth(&c, move |t| {
                    let (c, s, b) = (cc.clone(), sha2.clone(), bytes.clone());
                    async move { c.upload_cover(&t, &s, b).await }
                })
                .await
                {
                    Ok(()) => {
                        st.uploaded_covers.insert(sha.clone());
                    }
                    Err(e) if is_signed_out(&e) => return Err(e),
                    Err(e) => tracing::info!("cover upload skipped: {e:#}"),
                }
            }

            // Read the account before uploading guest/default values on a new
            // device. The following round pushes the remaining local additions.
            let changes = if st.pulled_once { st.enqueue(&snap, now) } else { Vec::new() };
            let wires = match push_mode(&st, &keys) {
                PushMode::Encrypt => {
                    let mut items = changes;
                    items.extend(crate::e2e_sync::repush(&st, &snap, now));
                    crate::e2e_sync::outgoing(items, &keys.keys, &st.e2e)
                }
                PushMode::Hold => Vec::new(),
                PushMode::Plain => crate::e2e_sync::plain_wires(changes),
            };
            require_session(&acc)?;
            st.save().context("persist sync outbox before sending")?;
            *sync_state().lock().unwrap() = Some(st.clone());
            for batch in crate::e2e_sync::batches(wires, 200, 512 * 1024) {
                let items: Vec<SyncItem> = batch.iter().map(|w| w.item.clone()).collect();
                let r = with_auth(&c, |t| {
                    let (c, items, acc) = (c.clone(), items.clone(), acc.clone());
                    async move { require_session(&acc)?; c.sync_push(&t, items).await }
                })
                .await?;
                round.pushed += r.accepted as usize;
                let stale: std::collections::BTreeSet<(String, String)> = r.stale.iter().map(|i| (i.kind.clone(), i.key.clone())).collect();
                let accepted: Vec<_> = batch.into_iter().filter(|w| !stale.contains(&(w.item.kind.clone(), w.item.key.clone()))).collect();
                crate::e2e_sync::accepted(&mut st, &accepted);
                round.items.extend(crate::e2e_sync::incoming(r.stale, &keys.keys, &mut st, &mut notices));
                require_session(&acc)?;
                st.save().context("persist sync acknowledgement")?;
                *sync_state().lock().unwrap() = Some(st.clone());
            }

            let mut pulled = Vec::new();
            loop {
                let since = st.last_rev;
                let r = with_auth(&c, |t| {
                    let (c, acc) = (c.clone(), acc.clone());
                    async move { require_session(&acc)?; c.sync_pull(&t, since).await }
                })
                .await?;
                if r.reset {
                    // The server forgot old deletions: agree on everything again.
                    st.reset();
                    pulled.clear();
                    continue;
                }
                pulled.extend(r.items);
                st.last_rev = r.rev;
                if !r.more {
                    break;
                }
            }
            st.pulled_once = true;
            st.e2e.absorb_keyring(&pulled);

            // Playlists on the profile stay readable for the server (it shows them).
            if now.saturating_sub(st.e2e.public_checked) > PUBLIC_EVERY_MS {
                match with_auth(&c, |t| {
                    let c = c.clone();
                    async move { c.profile_settings(&t).await }
                })
                .await
                {
                    Ok(p) => {
                        st.e2e.public = p.playlists.into_iter().collect();
                        st.e2e.public_checked = now;
                    }
                    Err(e) if is_signed_out(&e) => return Err(e),
                    Err(e) => tracing::info!("profile playlists: {e:#}"),
                }
            }

            settle_keys(&c, &acc, &mut st, &mut keys, official, now, &mut round).await?;
            let mut items = if keys.current().is_some() { crate::e2e_sync::take_locked(&mut st) } else { Vec::new() };
            items.extend(pulled);
            round.items.extend(crate::e2e_sync::incoming(items, &keys.keys, &mut st, &mut notices));
            if notices.contains(&crate::e2e_sync::Notice::Tampered) {
                crate::privacy::record(c.base_url(), &acc.account.id, true, "tampered", "");
            }

            // Pictures of playlists from other devices.
            let _ = std::fs::create_dir_all(sync::synced_covers_dir());
            for i in &round.items {
                let Some(sha) = i.value.as_ref().and_then(|v| v["cover"].as_str()).filter(|_| i.kind == glyph_protocol::sync_kind::PLAYLIST) else { continue };
                if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) || sync::synced_cover(sha).is_some() || snap.cover_files.contains_key(sha) {
                    continue;
                }
                let s = sha.to_string();
                match with_auth(&c, |t| {
                    let (c, s) = (c.clone(), s.clone());
                    async move { c.download_cover(&t, &s).await }
                })
                .await
                {
                    Ok(bytes) => {
                        let ext = sync::image_ext(&bytes);
                        let _ = paths::write_atomic(&sync::synced_covers_dir().join(format!("{sha}.{ext}")), &bytes);
                    }
                    Err(e) => tracing::info!("cover download skipped: {e:#}"),
                }
            }

            require_session(&acc)?;
            st.save().context("save sync state")?;
            *sync_state().lock().unwrap() = Some(st);
            Ok(round)
        })
        .await?
    }

    /// The profile's playlists changed: they go readable (or encrypted
    /// again) with the next sync round.
    pub fn sync_profile_playlists(&self, ids: &[String]) {
        let Some(acc) = self.account() else { return };
        let mut g = sync_state().lock().unwrap();
        let st = g.get_or_insert_with(|| SyncState::load(&acc.account.id));
        st.e2e.public = ids.iter().cloned().collect();
        st.e2e.public_checked = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let _ = st.save();
    }

    /// Playlists are shown on the profile (they stay readable for the server).
    pub fn profile_shows_playlists(&self) -> bool {
        let Some(acc) = self.account() else { return false };
        let mut g = sync_state().lock().unwrap();
        !g.get_or_insert_with(|| SyncState::load(&acc.account.id)).e2e.public.is_empty()
    }

    /// Encryption state of the library on this device (for the transparency page).
    pub fn library_crypto(&self) -> crate::privacy::Library {
        let Some(acc) = self.account() else { return crate::privacy::Library::Guest };
        let keys = load_keys(&acc.account.id);
        let mut g = sync_state().lock().unwrap();
        let st = g.get_or_insert_with(|| SyncState::load(&acc.account.id));
        match st.e2e.main() {
            None => crate::privacy::Library::Starting,
            Some(m) => match keys.current() {
                Some(k) if k.kid() == m.kid => {
                    if m.escrow.is_some() || st.e2e.escrowed.contains(&m.kid) { crate::privacy::Library::Escrow } else { crate::privacy::Library::Encrypted }
                }
                _ => crate::privacy::Library::Locked,
            },
        }
    }

    /// Key for the synced API keys: from the library's data key, and only
    /// while the server cannot read that key (the account has a password).
    pub fn settings_vault(&self) -> Option<Vault> {
        (self.library_crypto() == crate::privacy::Library::Encrypted).then(|| {
            let acc = self.account()?;
            load_keys(&acc.account.id).current().map(|k| k.vault())
        })?
    }

    /// The library is end-to-end encrypted and its key has not reached this device yet.
    pub fn library_locked(&self) -> bool {
        self.library_crypto() == crate::privacy::Library::Locked
    }

    /// Main-thread half for settings: apply remote settings and keys to `cfg`.
    pub fn sync_apply_settings(&self, cfg: &mut Config, round: &SyncRound) -> SettingsApplied {
        let Some(acc) = self.account() else { return SettingsApplied::default() };
        if round.owner.as_ref() != Some(&acc) { return SettingsApplied::default(); }
        let vault = self.settings_vault();
        // Copies from before the data key, sealed with the old password key.
        let legacy = get_vault(&acc.account.id);
        let mut g = sync_state().lock().unwrap();
        let st = g.get_or_insert_with(|| SyncState::load(&acc.account.id));
        let r = settings_sync::apply(cfg, st, &round.items, vault.as_ref(), legacy.as_ref());
        if let Err(e) = st.save() {
            tracing::warn!("sync state: {e}");
        }
        r
    }

    /// The account password on a device signed in without it: checks it with
    /// the server and keeps the key that opens the library's data key; the
    /// next sync round uses it (and replaces a key the server kept).
    pub async fn account_unlock_keys(&self, password: &str) -> Result<()> {
        let acc = self.account().ok_or_else(|| anyhow!(SIGNED_OUT))?;
        let (c, password) = (self.account_client()?, password.to_string());
        self.spawn(async move {
            let salt = match c.password_salt(&acc.account.handle).await {
                Ok(s) => checked_salt(&s.salt).inspect_err(|_| crate::privacy::record(c.base_url(), &acc.account.id, true, "bad_salt", ""))?,
                Err(e) if matches!(server_error(&e), Some((404, _))) => return Err(anyhow!("no_password")),
                Err(e) => return Err(e),
            };
            check_salt_after_sign_in(c.base_url(), &salt, &acc.account);
            let key = b64(&derive_async(password.clone(), salt).await?);
            with_auth(&c, |t| {
                let (c, k) = (c.clone(), key.clone());
                async move { c.password_check(&t, &k).await }
            })
            .await
            .map_err(|e| if matches!(server_error(&e), Some((403, _))) { anyhow!("wrong_password") } else { e })?;
            put_pw_key(&acc.account.id, &derive_pw_key(password, acc.account.id.clone()).await?, false);
            Ok(())
        })
        .await?
    }

    /// Main-thread half of a sync round: apply remote items to the library.
    pub fn sync_apply(&self, lib: &mut Library, round: &SyncRound) -> Applied {
        let downloaded = self.downloaded_tracks();
        let Some(acc) = self.account() else { return Applied::default() };
        if round.owner.as_ref() != Some(&acc) { return Applied::default(); }
        let mut g = sync_state().lock().unwrap();
        let st = g.get_or_insert_with(|| SyncState::load(&acc.account.id));
        let applied = sync::apply(lib, st, &round.items, &downloaded);
        if let Err(e) = st.save() {
            tracing::warn!("sync state: {e}");
        }
        applied
    }
}

// ------------------------------------------------------------------ MCP relay

impl Backend {
    /// A new MCP token for AI assistants reaching this account's app
    /// through the server.
    pub async fn mcp_token(&self) -> Result<glyph_protocol::mcp::TokenResponse> {
        self.profile_call(|c, t| async move { c.mcp_token(&t).await }).await
    }

    /// Waits (long poll) for tool calls queued on the server.
    pub async fn mcp_pull(&self) -> Result<glyph_protocol::mcp::PullResponse> {
        self.profile_call(|c, t| async move { c.mcp_pull(&t).await }).await
    }

    pub async fn mcp_approve(&self, d: glyph_protocol::mcp::Decision) -> Result<()> {
        self.profile_call(move |c, t| {
            let d = d.clone();
            async move { c.mcp_approve(&t, &d).await }
        })
        .await
    }

    pub async fn mcp_result(&self, r: glyph_protocol::mcp::ResultRequest) -> Result<()> {
        self.profile_call(move |c, t| {
            let r = r.clone();
            async move { c.mcp_result(&t, &r).await }
        })
        .await
    }
}

// ------------------------------------------------------------------ jam

impl Backend {
    /// Starts a jam hosted by the signed-in account (ends an older one).
    pub async fn jam_start(&self) -> Result<glyph_protocol::jam::State> {
        self.profile_call(|c, t| async move { c.jam_start(&t).await }).await
    }

    /// The jam's state once its revision differs from `rev` (long poll).
    pub async fn jam_state(&self, code: &str, rev: u64) -> Result<glyph_protocol::jam::State> {
        let code = code.to_string();
        self.profile_call(move |c, t| {
            let code = code.clone();
            async move { c.jam_state(&t, &code, rev).await }
        })
        .await
    }

    /// In (the state), or waiting for the host to let us in.
    pub async fn jam_join(&self, code: &str) -> Result<glyph_protocol::jam::JoinResponse> {
        let code = code.to_string();
        self.profile_call(move |c, t| {
            let code = code.clone();
            async move { c.jam_join(&t, &code).await }
        })
        .await
    }

    /// One clock sample: (local ms sent, server ms, local ms received).
    pub async fn jam_time(&self, code: &str) -> Result<(u64, u64, u64)> {
        let code = code.to_string();
        self.profile_call(move |c, t| {
            let code = code.clone();
            async move {
                let unix = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
                let sent = unix();
                let r = c.jam_time(&t, &code).await?;
                Ok((sent, r.server_ms, unix()))
            }
        })
        .await
    }

    pub async fn jam_admit(&self, code: &str, handle: &str, allow: bool) -> Result<()> {
        let (code, handle) = (code.to_string(), handle.to_string());
        self.profile_call(move |c, t| {
            let (code, handle) = (code.clone(), handle.clone());
            async move { c.jam_admit(&t, &code, &handle, allow).await }
        })
        .await
    }

    pub async fn jam_claim(&self, code: &str) -> Result<glyph_protocol::jam::State> {
        let code = code.to_string();
        self.profile_call(move |c, t| {
            let code = code.clone();
            async move { c.jam_claim(&t, &code).await }
        })
        .await
    }

    /// Leaves (or, as the host, ends) the jam.
    pub async fn jam_leave(&self, code: &str) -> Result<()> {
        let code = code.to_string();
        self.profile_call(move |c, t| {
            let code = code.clone();
            async move { c.jam_leave(&t, &code).await }
        })
        .await
    }

    pub async fn jam_op(&self, code: &str, op: glyph_protocol::jam::Op) -> Result<()> {
        let code = code.to_string();
        self.profile_call(move |c, t| {
            let (code, op) = (code.clone(), op.clone());
            async move { c.jam_op(&t, &code, &op).await }
        })
        .await
    }

    /// The members' requests waiting for the host (long poll; `now`: only
    /// what is there).
    pub async fn jam_ops(&self, code: &str, now: bool) -> Result<glyph_protocol::jam::OpsResponse> {
        let code = code.to_string();
        self.profile_call(move |c, t| {
            let code = code.clone();
            async move { c.jam_ops(&t, &code, now).await }
        })
        .await
    }

    /// The jam's push socket (see [`ServerClient::jam_live`]).
    pub async fn jam_live(&self, code: &str) -> Result<tokio::sync::mpsc::Receiver<crate::server_client::LiveEvent>> {
        let code = code.to_string();
        self.profile_call(move |c, t| {
            let code = code.clone();
            async move { c.jam_live(&t, &code).await }
        })
        .await
    }

    pub async fn jam_publish(&self, state: glyph_protocol::jam::State) -> Result<()> {
        self.profile_call(move |c, t| {
            let s = state.clone();
            async move { c.jam_publish(&t, &s).await }
        })
        .await
    }

    pub async fn jam_now(&self, code: &str, now: glyph_protocol::jam::Now) -> Result<()> {
        let code = code.to_string();
        self.profile_call(move |c, t| {
            let (code, now) = (code.clone(), now.clone());
            async move { c.jam_now(&t, &code, &now).await }
        })
        .await
    }

    pub async fn jam_kick(&self, code: &str, handle: &str) -> Result<()> {
        let (code, handle) = (code.to_string(), handle.to_string());
        self.profile_call(move |c, t| {
            let (code, handle) = (code.clone(), handle.clone());
            async move { c.jam_kick(&t, &code, &handle).await }
        })
        .await
    }
}

// ------------------------------------------------------------------ profiles

impl Backend {
    /// One signed-in call on the backend runtime.
    async fn profile_call<T, F, Fut>(&self, f: F) -> Result<T>
    where
        F: Fn(ServerClient, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T>> + Send,
        T: Send + 'static,
    {
        let c = self.account_client()?;
        let expected = self.account().ok_or_else(|| anyhow!(SIGNED_OUT))?;
        self.spawn(async move {
            with_auth(&c, |t| {
                let expected = expected.clone();
                let future = f(c.clone(), t);
                async move { require_session(&expected)?; future.await }
            }).await
        }).await?
    }

    pub async fn profile_settings(&self) -> Result<ProfileSettings> {
        let p = self.profile_call(|c, t| async move { c.profile_settings(&t).await }).await?;
        self.sync_profile_playlists(&p.playlists);
        Ok(p)
    }

    pub async fn profile_update(&self, patch: ProfilePatch) -> Result<ProfileSettings> {
        let p = self
            .profile_call(move |c, t| {
                let p = patch.clone();
                async move { c.profile_patch(&t, &p).await }
            })
            .await?;
        self.sync_profile_playlists(&p.playlists);
        Ok(p)
    }

    /// Sets (or with `None` removes) the picture or the banner.
    pub async fn profile_set_image(&self, banner: bool, bytes: Option<Vec<u8>>) -> Result<ProfileSettings> {
        self.profile_call(move |c, t| {
            let b = bytes.clone();
            async move { c.profile_image(&t, banner, b).await }
        })
        .await
    }

    /// Songs played last, for profiles that show them.
    pub async fn profile_push_recent(&self, tracks: Vec<Track>) -> Result<()> {
        self.profile_call(move |c, t| {
            let tr = tracks.clone();
            async move { c.profile_recent(&t, tr).await }
        })
        .await
    }

    pub async fn profile(&self, handle: &str) -> Result<PublicProfile> {
        let h = handle.to_string();
        self.profile_call(move |c, t| {
            let h = h.clone();
            async move { c.profile(&t, &h).await }
        })
        .await
    }

    pub async fn profile_playlist(&self, handle: &str, id: &str) -> Result<ProfilePlaylistSongs> {
        let (h, id) = (handle.to_string(), id.to_string());
        self.profile_call(move |c, t| {
            let (h, id) = (h.clone(), id.clone());
            async move { c.profile_playlist(&t, &h, &id).await }
        })
        .await
    }

    /// A profile picture, banner or playlist cover as a local file (kept with
    /// the synced covers, by hash).
    pub async fn profile_image(&self, handle: &str, sha256: &str) -> Result<PathBuf> {
        if let Some(p) = sync::synced_cover(sha256) {
            return Ok(p);
        }
        let (h, sha) = (handle.to_string(), sha256.to_string());
        let bytes = self
            .profile_call(move |c, t| {
                let (h, sha) = (h.clone(), sha.clone());
                async move { c.profile_image_bytes(&t, &h, &sha).await }
            })
            .await?;
        let _ = std::fs::create_dir_all(sync::synced_covers_dir());
        let path = sync::synced_covers_dir().join(format!("{sha256}.{}", sync::image_ext(&bytes)));
        paths::write_atomic(&path, &bytes)?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_guest_merge_survives_restart_without_exposing_guest_data_to_account() {
        let _env = crate::paths::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        struct Restore(Vec<(&'static str, Option<std::ffi::OsString>)>);
        impl Drop for Restore {
            fn drop(&mut self) { for (key, value) in &self.0 { match value { Some(v) => std::env::set_var(key, v), None => std::env::remove_var(key) } } }
        }
        let _restore = Restore(["GLYPH_DATA_DIR", "GLYPH_CONFIG", "GLYPH_DEMO"].into_iter().map(|key| (key, std::env::var_os(key))).collect());
        let dir = tempfile::tempdir().unwrap();
        std::env::remove_var("GLYPH_DEMO");
        std::env::set_var("GLYPH_DATA_DIR", dir.path().join("data"));
        std::env::set_var("GLYPH_CONFIG", dir.path().join("config.toml"));
        let stored = Stored { server: crate::config::DEFAULT_SERVER_URL.into(), device_id: "new-device".into(), account: Account {
            id: "account".into(), handle: "owner".into(), device_limit: 5, created_at: 0, has_password: false,
        }};
        paths::write_json(&stored_path(), &stored).unwrap();
        paths::write_json(&guest_merge_marker(), &serde_json::json!({"server": stored.server})).unwrap();
        let mut guest = Library::default();
        guest.toggle_like(&crate::dedupe::test_track(glyph_protocol::ProviderId::Audius, "guest", "Guest", "Artist", Some(100)));
        guest.save_to(&Library::path()).unwrap();
        let restarted = Library::load().unwrap();
        assert_eq!(restarted.liked.len(), 1);
        assert_eq!(restarted.storage_path.0, Some(Library::guest_path()));
        assert!(!library_path_for(&stored).exists());
        // "Use account data only" creates its empty cache, keeping guest data.
        Library::default().save_to(&library_path_for(&stored)).unwrap();
        std::fs::remove_file(guest_merge_marker()).unwrap();
        assert!(Library::load().unwrap().liked.is_empty());
        assert_eq!(Library::load_from(&Library::guest_path()).unwrap().liked.len(), 1);
    }

    #[test]
    fn account_cache_paths_separate_servers_and_ignore_untrusted_path_segments() {
        let _env = crate::paths::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let a = Stored { server: "https://one.example".into(), device_id: "device".into(), account: Account {
            id: "../../account".into(), handle: "owner".into(), device_limit: 5, created_at: 0, has_password: false,
        }};
        let mut b = a.clone(); b.server = "https://two.example".into();
        assert_ne!(library_path_for(&a), library_path_for(&b));
        b = a.clone(); b.account.id = "other".into();
        assert_ne!(library_path_for(&a), library_path_for(&b));
        assert!(library_path_for(&a).starts_with(paths::data_dir().join("accounts")));
        with_test_access("https://one.example", "token", || {
            assert!(require_session(&a).is_err());
            // A stale callback must not clear a newer session.
            drop_local_if_current(Some(&a));
            assert!(session().lock().unwrap().is_some());
        });
    }

    #[test]
    fn key_derivation_is_stable_and_salted() {
        let a = derive_key("correct horse battery", &[1; 16]).unwrap();
        assert_eq!(a.len(), password_kdf::KEY_LEN);
        assert_eq!(a, derive_key("correct horse battery", &[1; 16]).unwrap());
        assert_ne!(a, derive_key("correct horse battery", &[2; 16]).unwrap());
        assert_ne!(a, derive_key("correct horse batterz", &[1; 16]).unwrap());
        assert_eq!(password_problem("short"), Some("too_short"));
        assert_eq!(password_problem("long enough pw"), None);
    }
}

#[cfg(test)]
mod kdf_vector {
    /// Same vector as the browser login page (hash-wasm argon2id): every
    /// client must derive the identical key or password sign-in breaks.
    #[test]
    fn matches_the_web_page() {
        let k = super::derive_key("correct horse battery", &[1; 16]).unwrap();
        let hex: String = k.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "dfada508e6511ebcce7457b66284b157a5f9a3890dbfb3ef4190cbc4280a8120");
    }
}
