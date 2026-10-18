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

fn stored_path() -> PathBuf {
    paths::data_dir().join("account.json")
}

fn load_stored() -> Option<Stored> {
    serde_json::from_slice(&std::fs::read(stored_path()).ok()?).ok()
}

// ------------------------------------------------------------------ secrets

#[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "linux"))]
const KEYCHAIN_SERVICE: &str = "glyph";

fn fallback_path(device_id: &str) -> PathBuf {
    paths::data_dir().join(format!("account_token_{device_id}"))
}

#[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "linux"))]
fn entry(device_id: &str) -> Option<keyring::Entry> {
    keyring::Entry::new(KEYCHAIN_SERVICE, &format!("account:{device_id}")).ok()
}

fn put_refresh(device_id: &str, token: &str) {
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
    #[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "linux"))]
    if let Some(t) = entry(device_id).and_then(|e| e.get_password().ok()) {
        return Some(t);
    }
    std::fs::read_to_string(fallback_path(device_id)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn forget_refresh(device_id: &str) {
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
    #[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "linux"))]
    if let Some(t) = keyring::Entry::new(KEYCHAIN_SERVICE, &format!("settings-key:{account_id}")).ok().and_then(|e| e.get_password().ok()) {
        return unb64(&t).and_then(|b| Vault::from_bytes(&b));
    }
    let t = std::fs::read_to_string(vault_fallback(account_id)).ok()?;
    unb64(t.trim()).and_then(|b| Vault::from_bytes(&b))
}

fn forget_vault(account_id: &str) {
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
    let stored = Stored { server: server.to_string(), account: s.account.clone(), device_id: s.device_id.clone() };
    put_refresh(&s.device_id, &s.refresh_token);
    if let Err(e) = paths::write_json(&stored_path(), &stored) {
        tracing::warn!("could not save account: {e}");
    }
    let exp = Instant::now() + Duration::from_secs(s.access_expires_in_s.saturating_sub(60));
    *session().lock().unwrap() = Some(Live { stored, access: Some((s.access_token.clone(), exp)) });
}

/// Forget the local session (tokens, state). The library stays.
fn drop_local() {
    if let Some(live) = session().lock().unwrap().take() {
        forget_refresh(&live.stored.device_id);
        forget_vault(&live.stored.account.id);
    }
    let _ = std::fs::remove_file(stored_path());
    let _ = std::fs::remove_file(SyncState::path());
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
    let (device_id, cached) = {
        let g = session().lock().unwrap();
        let live = g.as_ref().ok_or_else(|| anyhow!(SIGNED_OUT))?;
        if live.stored.server != client.base_url() {
            return Err(anyhow!(SIGNED_OUT));
        }
        (live.stored.device_id.clone(), live.access.clone())
    };
    if let (false, Some((tok, exp))) = (force, cached) {
        if exp > Instant::now() {
            return Ok(tok);
        }
    }
    let refresh = tokio::task::spawn_blocking(move || get_refresh(&device_id)).await?
        .ok_or_else(|| anyhow!(SIGNED_OUT))?;
    match client.refresh_session(&refresh).await {
        Ok(s) => {
            let (base, fresh) = (client.base_url().to_string(), s.clone());
            tokio::task::spawn_blocking(move || adopt(&base, &fresh)).await?;
            Ok(s.access_token)
        }
        Err(e) if matches!(server_error(&e), Some((401, _))) => {
            drop_local();
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
    let tok = access_token(client, false).await?;
    match f(tok).await {
        Err(e) if matches!(server_error(&e), Some((401, "token_expired"))) => f(access_token(client, true).await?).await,
        Err(e) if matches!(server_error(&e), Some((401, _))) => {
            drop_local();
            Err(anyhow!(SIGNED_OUT))
        }
        r => r,
    }
}

impl Backend {
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
        let server = match inner.server.clone() {
            Some(s) => s,
            // Links always live on the public server.
            None => ServerClient::new(crate::config::DEFAULT_SERVER_URL, inner.http.clone(), crate::server_client::install_id()),
        };
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
        let c = self.account_client()?;
        self.spawn(async move {
            let r = with_auth(&c, |t| {
                let c = c.clone();
                async move { c.sign_out_device(&t).await }
            })
            .await;
            drop_local();
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
        let (c, h) = (self.account_client()?, confirm_handle.to_string());
        self.spawn(async move {
            with_auth(&c, |t| {
                let (c, h) = (c.clone(), h.clone());
                async move { c.account_delete(&t, &h).await }
            })
            .await?;
            drop_local();
            Ok(())
        })
        .await?
    }

    /// Songs this device should keep offline (downloaded + requested by sync).
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

            let changes = st.changes(&snap, now);
            for chunk in sync::chunks(changes, 200, 512 * 1024) {
                let r = with_auth(&c, |t| {
                    let (c, items) = (c.clone(), chunk.clone());
                    async move { c.sync_push(&t, items).await }
                })
                .await?;
                round.pushed += r.accepted as usize;
                let stale: std::collections::BTreeSet<(String, String)> = r.stale.iter().map(|i| (i.kind.clone(), i.key.clone())).collect();
                let accepted: Vec<SyncItem> = chunk.into_iter().filter(|i| !stale.contains(&(i.kind.clone(), i.key.clone()))).collect();
                st.mark_pushed(&accepted);
                round.items.extend(r.stale);
            }

            loop {
                let since = st.last_rev;
                let r = with_auth(&c, |t| {
                    let c = c.clone();
                    async move { c.sync_pull(&t, since).await }
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
        let mut g = sync_state().lock().unwrap();
        let st = g.get_or_insert_with(|| SyncState::load(&acc.account.id));
        let applied = sync::apply(lib, st, &round.items, &downloaded);
        if let Err(e) = st.save() {
            tracing::warn!("sync state: {e}");
        }
        applied
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
        self.spawn(async move { with_auth(&c, |t| f(c.clone(), t)).await }).await?
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
