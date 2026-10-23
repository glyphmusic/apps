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

// Settings key (see `settings_sync`): keychain entry per account, file fallback.

fn vault_fallback(account_id: &str) -> PathBuf {
    paths::data_dir().join(format!("settings_key_{account_id}"))
}

fn put_vault(account_id: &str, v: &Vault) {
    let text = b64(v.bytes());
    if secret_store().is_some_and(|s| s.set(&format!("settings-key:{account_id}"), &text)) {
        let _ = std::fs::remove_file(vault_fallback(account_id));
        return;
    }
    #[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "linux"))]
    if let Ok(e) = keyring::Entry::new(KEYCHAIN_SERVICE, &format!("settings-key:{account_id}")) {
        if e.set_password(&text).is_ok() {
            let _ = std::fs::remove_file(vault_fallback(account_id));
            return;
        }
    }
    if let Err(e) = paths::write_atomic(&vault_fallback(account_id), text.as_bytes()) {
        tracing::warn!("could not store the settings key: {e}");
    }
}

fn get_vault(account_id: &str) -> Option<Vault> {
    if let Some(t) = secret_store().and_then(|s| s.get(&format!("settings-key:{account_id}"))) {
        return unb64(&t).and_then(|b| Vault::from_bytes(&b));
    }
    #[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "linux"))]
    if let Some(t) = keyring::Entry::new(KEYCHAIN_SERVICE, &format!("settings-key:{account_id}")).ok().and_then(|e| e.get_password().ok()) {
        return unb64(&t).and_then(|b| Vault::from_bytes(&b));
    }
    let t = std::fs::read_to_string(vault_fallback(account_id)).ok()?;
    unb64(t.trim()).and_then(|b| Vault::from_bytes(&b))
}

fn forget_vault(account_id: &str) {
    if let Some(s) = secret_store() {
        s.delete(&format!("settings-key:{account_id}"));
    }
    #[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "linux"))]
    if let Ok(e) = keyring::Entry::new(KEYCHAIN_SERVICE, &format!("settings-key:{account_id}")) {
        let _ = e.delete_credential();
    }
    let _ = std::fs::remove_file(vault_fallback(account_id));
}

async fn derive_vault(password: String, account_id: String) -> Result<Vault> {
    tokio::task::spawn_blocking(move || Vault::derive(&password, &account_id)).await.map_err(|e| anyhow!("key derivation: {e}"))?
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
        forget_vault(&live.stored.account.id);
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
        self.spawn(async move { c.health().await.map(|h| h.accounts).unwrap_or(false) }).await.unwrap_or(false)
    }

    pub async fn account_login_start(&self) -> Result<LoginStartResponse> {
        let c = self.account_client()?;
        self.spawn(async move { c.login_start(&device_name(), platform()).await }).await?
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
            put_vault(&id, &derive_vault(password, id.clone()).await?);
            Ok(r.recovery_code)
        })
        .await?
    }

    /// Sign in with handle + password.
    pub async fn account_password_login(&self, handle: &str, password: &str) -> Result<Account> {
        let (c, handle, password) = (self.account_client()?, handle.to_string(), password.to_string());
        self.spawn(async move {
            let salt = match c.password_salt(&handle).await {
                Ok(s) => unb64(&s.salt).ok_or_else(|| anyhow!("bad salt"))?,
                // Same answer as a wrong password: do not tell which part was wrong.
                Err(e) if matches!(server_error(&e), Some((404, _))) => return Err(anyhow!("login_failed")),
                Err(e) => return Err(e),
            };
            let key = derive_async(password.clone(), salt).await?;
            let body = PasswordLoginRequest { handle, key: b64(&key), device_name: device_name(), platform: platform().into() };
            let s = c.password_login(&body).await?;
            drop_local();
            adopt(c.base_url(), &s);
            put_vault(&s.account.id, &derive_vault(password, s.account.id.clone()).await?);
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
            // New password, new settings key: the keys are re-encrypted with
            // it on the next sync; other devices ask for the new password.
            put_vault(&a.id, &derive_vault(password, a.id.clone()).await?);
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

    /// Approve a pairing code shown on a new device.
    pub async fn account_approve(&self, user_code: &str) -> Result<ApproveResponse> {
        let (c, code) = (self.account_client()?, user_code.to_string());
        self.spawn(async move {
            with_auth(&c, |t| {
                let (c, code) = (c.clone(), code.clone());
                async move { c.approve_login(&t, &code).await }
            })
            .await
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
    pub async fn sync_exchange(&self, snap: Snapshot, settings: Vec<(Key, serde_json::Value)>) -> Result<SyncRound> {
        let acc = self.account().ok_or_else(|| anyhow!(SIGNED_OUT))?;
        let c = self.account_client()?;
        self.spawn(async move {
            require_session(&acc)?;
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
            require_session(&acc)?;
            st.save().context("persist sync outbox before sending")?;
            *sync_state().lock().unwrap() = Some(st.clone());
            for chunk in sync::chunks(changes, 200, 512 * 1024) {
                let r = with_auth(&c, |t| {
                    let (c, items, acc) = (c.clone(), chunk.clone(), acc.clone());
                    async move { require_session(&acc)?; c.sync_push(&t, items).await }
                })
                .await?;
                round.pushed += r.accepted as usize;
                let stale: std::collections::BTreeSet<(String, String)> = r.stale.iter().map(|i| (i.kind.clone(), i.key.clone())).collect();
                let accepted: Vec<SyncItem> = chunk.into_iter().filter(|i| !stale.contains(&(i.kind.clone(), i.key.clone()))).collect();
                st.mark_pushed(&accepted);
                require_session(&acc)?;
                st.save().context("persist sync acknowledgement")?;
                *sync_state().lock().unwrap() = Some(st.clone());
                round.items.extend(r.stale);
            }

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
                    continue;
                }
                round.items.extend(r.items);
                st.last_rev = r.rev;
                if !r.more {
                    break;
                }
            }
            st.pulled_once = true;

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

    /// Key for the encrypted API keys, if this device has one.
    pub fn settings_vault(&self) -> Option<Vault> {
        get_vault(&self.account()?.account.id)
    }

    /// Encrypted keys arrived that this device cannot open yet.
    pub fn settings_locked(&self) -> bool {
        let Some(acc) = self.account() else { return false };
        let mut g = sync_state().lock().unwrap();
        g.get_or_insert_with(|| SyncState::load(&acc.account.id)).locked_secret.is_some()
    }

    /// Main-thread half for settings: apply remote settings and keys to `cfg`.
    pub fn sync_apply_settings(&self, cfg: &mut Config, round: &SyncRound) -> SettingsApplied {
        let Some(acc) = self.account() else { return SettingsApplied::default() };
        if round.owner.as_ref() != Some(&acc) { return SettingsApplied::default(); }
        let vault = get_vault(&acc.account.id);
        let mut g = sync_state().lock().unwrap();
        let st = g.get_or_insert_with(|| SyncState::load(&acc.account.id));
        let r = settings_sync::apply(cfg, st, &round.items, vault.as_ref());
        if r.vault_outdated {
            forget_vault(&acc.account.id);
        }
        if let Err(e) = st.save() {
            tracing::warn!("sync state: {e}");
        }
        r
    }

    /// The account password on a device signed in without it: checks it with
    /// the server, keeps the settings key and returns it so the caller can
    /// open keys that are waiting ([`Backend::sync_unlock_settings`]).
    pub async fn account_unlock_keys(&self, password: &str) -> Result<()> {
        let acc = self.account().ok_or_else(|| anyhow!(SIGNED_OUT))?;
        let (c, password) = (self.account_client()?, password.to_string());
        self.spawn(async move {
            let salt = match c.password_salt(&acc.account.handle).await {
                Ok(s) => unb64(&s.salt).ok_or_else(|| anyhow!("bad salt"))?,
                Err(e) if matches!(server_error(&e), Some((404, _))) => return Err(anyhow!("no_password")),
                Err(e) => return Err(e),
            };
            let key = b64(&derive_async(password.clone(), salt).await?);
            with_auth(&c, |t| {
                let (c, k) = (c.clone(), key.clone());
                async move { c.password_check(&t, &k).await }
            })
            .await
            .map_err(|e| if matches!(server_error(&e), Some((403, _))) { anyhow!("wrong_password") } else { e })?;
            put_vault(&acc.account.id, &derive_vault(password, acc.account.id.clone()).await?);
            Ok(())
        })
        .await?
    }

    /// After [`Backend::account_unlock_keys`]: open keys that arrived while
    /// locked. `true` when `cfg` got new keys.
    pub fn sync_unlock_settings(&self, cfg: &mut Config) -> bool {
        let Some(acc) = self.account() else { return false };
        let Some(vault) = get_vault(&acc.account.id) else { return false };
        let mut g = sync_state().lock().unwrap();
        let st = g.get_or_insert_with(|| SyncState::load(&acc.account.id));
        let opened = match settings_sync::unlock(cfg, st, &vault) {
            Ok(o) => o,
            Err(_) => {
                // Older blob from before a password change: the next pull brings the current one.
                st.locked_secret = None;
                false
            }
        };
        let _ = st.save();
        opened
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
        self.profile_call(|c, t| async move { c.profile_settings(&t).await }).await
    }

    pub async fn profile_update(&self, patch: ProfilePatch) -> Result<ProfileSettings> {
        self.profile_call(move |c, t| {
            let p = patch.clone();
            async move { c.profile_patch(&t, &p).await }
        })
        .await
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
