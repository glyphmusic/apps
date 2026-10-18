//! Settings -> Account: optional sign-in (passkey in the browser or a pairing
//! code), devices with the server's device limit, and library sync. Design:
//! `docs/ACCOUNTS.md`. Without an account nothing here runs.

use crate::app::Glyph;
use crate::guard::guard_res;
use crate::i18n::{t, tf};
use crate::theme;
use crate::ui::input::TextInput;
use crate::ui::settings::{input_box, row, rows, section_label, segmented, white};
use crate::ui::widgets::*;
use glyph_core::account::{is_signed_out, Stored};
use glyph_protocol::{AccountPatch, DevicesResponse, LoginPollResponse, LoginStartResponse};
use gpui::{div, prelude::*, px, AnyElement, Context, Div, Entity, FontWeight, SharedString};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// How often a signed-in app syncs on its own.
const SYNC_EVERY: Duration = Duration::from_secs(5 * 60);
/// Quiet time after a library change before it is pushed.
const SYNC_DEBOUNCE: Duration = Duration::from_secs(2);
const POLL_EVERY: Duration = Duration::from_secs(2);

#[derive(Clone, Default)]
pub enum Login {
    #[default]
    Idle,
    Starting,
    /// Browser open / pairing code shown; polling.
    Waiting(LoginStartResponse),
    /// Signed in, but the device limit is reached.
    Full(LoginStartResponse),
    Failed(String),
}

#[derive(Default)]
pub struct AccountUi {
    /// Server offers accounts (`None` = not checked yet).
    pub available: Option<bool>,
    pub login: Login,
    pub devices: Option<DevicesResponse>,
    pub syncing: bool,
    pub last_sync: Option<Instant>,
    pub sync_error: Option<String>,
    /// Bumped on every library change; a debounced sync only runs if unchanged.
    pub sync_gen: u64,
    /// Feedback line under the actions (approve, rename, ...).
    pub notice: Option<(String, bool)>,
    pub recovery_code: Option<String>,
    /// The recovery code is shown in clear (hidden as dots until clicked).
    pub recovery_visible: bool,
    pub handle_input: Option<Entity<TextInput>>,
    pub code_input: Option<Entity<TextInput>>,
    pub delete_input: Option<Entity<TextInput>>,
    /// Signed-out form: "create account" instead of "sign in".
    pub register: bool,
    /// On the public server the browser page is the default sign-in; this
    /// shows the in-app form instead.
    pub in_app: bool,
    /// A password call is running (key derivation + request).
    pub busy: bool,
    pub user_input: Option<Entity<TextInput>>,
    pub pw_input: Option<Entity<TextInput>>,
    pub pw2_input: Option<Entity<TextInput>>,
    pub newpw_input: Option<Entity<TextInput>>,
    pub newpw2_input: Option<Entity<TextInput>>,
    /// Password to open synced API keys on a device signed in without it.
    pub unlock_input: Option<Entity<TextInput>>,
    /// This device holds the key for the encrypted API keys (cached; the
    /// keychain is not read on every frame).
    pub vault_ok: bool,
    loop_started: bool,
}

fn err_text(e: &anyhow::Error) -> String {
    if is_signed_out(e) {
        return t("account.signed_out").to_string();
    }
    match e.to_string().as_str() {
        "login_failed" => return t("account.err.login").to_string(),
        "password_too_short" => return t("account.err.pw_short").to_string(),
        "password_too_long" => return t("account.err.pw_long").to_string(),
        "wrong_password" => return t("account.err.wrong_password").to_string(),
        "no_password" => return t("account.keys.need_password").to_string(),
        _ => {}
    }
    match glyph_core::server_client::server_error(e).map(|(_, c)| c) {
        Some("login_failed") => t("account.err.login").to_string(),
        Some("device_limit") => t("account.err.device_limit").to_string(),
        Some("handle_taken") => t("account.err.taken").to_string(),
        Some("handle_invalid") => t("account.err.invalid").to_string(),
        Some("handle_reserved") => t("account.err.reserved").to_string(),
        Some("handle_cooldown") => t("account.err.cooldown").to_string(),
        Some("too_many_devices") => t("account.err.too_many").to_string(),
        Some("not_found") => t("account.err.code").to_string(),
        Some("rate_limited") => t("account.err.slow").to_string(),
        _ => t("account.err.generic").to_string(),
    }
}

impl Glyph {
    fn account_inputs(&mut self, cx: &mut Context<Self>) {
        let a = &mut self.account;
        if a.handle_input.is_none() {
            a.handle_input = Some(cx.new(|cx| TextInput::new("account.handle.placeholder", cx)));
            a.code_input = Some(cx.new(|cx| TextInput::new("account.pair.placeholder", cx)));
            a.delete_input = Some(cx.new(|cx| TextInput::new("account.delete.placeholder", cx)));
            a.user_input = Some(cx.new(|cx| TextInput::new("account.form.handle", cx)));
            a.pw_input = Some(cx.new(|cx| TextInput::new("account.form.password", cx).masked()));
            a.pw2_input = Some(cx.new(|cx| TextInput::new("account.form.repeat", cx).masked()));
            a.newpw_input = Some(cx.new(|cx| TextInput::new("account.form.password", cx).masked()));
            a.newpw2_input = Some(cx.new(|cx| TextInput::new("account.form.repeat", cx).masked()));
            a.unlock_input = Some(cx.new(|cx| TextInput::new("account.form.password", cx).masked()));
        }
    }

    fn read(input: &Option<Entity<TextInput>>, cx: &Context<Self>) -> String {
        input.as_ref().map(|i| i.read(cx).value.clone()).unwrap_or_default()
    }

    fn clear(input: &Option<Entity<TextInput>>, cx: &mut Context<Self>) {
        if let Some(i) = input.clone() {
            i.update(cx, |i, cx| i.set_value("", cx));
        }
    }

    pub fn account_set_mode(&mut self, register: bool, cx: &mut Context<Self>) {
        self.account.register = register;
        self.account.notice = None;
        self.account.login = Login::Idle;
        cx.notify();
    }

    /// Sign in or create an account with handle + password (form on the page).
    pub fn account_password_submit(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        if self.account.busy {
            return;
        }
        let handle = Self::read(&self.account.user_input, cx).trim().to_string();
        let pw = Self::read(&self.account.pw_input, cx);
        let register = self.account.register;
        if handle.is_empty() || pw.is_empty() {
            self.account.login = Login::Failed(t("account.err.fill").to_string());
            cx.notify();
            return;
        }
        if register && pw != Self::read(&self.account.pw2_input, cx) {
            self.account.login = Login::Failed(t("account.err.pw_mismatch").to_string());
            cx.notify();
            return;
        }
        self.account.busy = true;
        self.account.login = Login::Idle;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = if register {
                b.account_password_signup(&handle, &pw).await.map(Some)
            } else {
                b.account_password_login(&handle, &pw).await.map(|_| None)
            };
            this.update(cx, |s, cx| {
                s.account.busy = false;
                match r {
                    Ok(recovery) => {
                        Self::clear(&s.account.pw_input, cx);
                        Self::clear(&s.account.pw2_input, cx);
                        s.account.recovery_code = recovery;
                        s.account.recovery_visible = false;
                        let handle = s.signed_in().map(|a| a.account.handle).unwrap_or_default();
                        s.toast(tf("account.toast.signed_in", &[("handle", &handle)]).to_string());
                        s.account_page_opened(cx);
                        s.sync_now(cx);
                    }
                    Err(e) => s.account.login = Login::Failed(err_text(&e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Enter the account password once so synced API keys can be opened here.
    pub fn account_unlock_keys(&mut self, cx: &mut Context<Self>) {
        let pw = Self::read(&self.account.unlock_input, cx);
        if pw.is_empty() {
            return;
        }
        self.account_call(
            cx,
            move |b| Box::pin(async move { b.account_unlock_keys(&pw).await }),
            |s, _, cx| {
                Self::clear(&s.account.unlock_input, cx);
                s.account.vault_ok = true;
                if let Some(b) = s.backend.clone() {
                    let mut cfg = s.config.clone();
                    if b.sync_unlock_settings(&mut cfg) {
                        s.adopt_synced_config(cfg, true, cx);
                    }
                }
                s.account.notice = Some((t("account.keys.unlocked").to_string(), true));
                s.sync_now(cx);
            },
        );
    }

    pub fn account_change_password(&mut self, cx: &mut Context<Self>) {
        let pw = Self::read(&self.account.newpw_input, cx);
        if pw != Self::read(&self.account.newpw2_input, cx) {
            self.account.notice = Some((t("account.err.pw_mismatch").to_string(), false));
            cx.notify();
            return;
        }
        self.account_call(
            cx,
            move |b| Box::pin(async move { b.account_set_password(&pw).await }),
            |s, _, cx| {
                Self::clear(&s.account.newpw_input, cx);
                Self::clear(&s.account.newpw2_input, cx);
                s.account.notice = Some((t("account.password.saved").to_string(), true));
            },
        );
    }

    pub fn account_link_passkey(&mut self, cx: &mut Context<Self>) {
        self.account_call(
            cx,
            |b| Box::pin(async move { b.account_link_passkey().await }),
            |s, r, cx| {
                cx.open_url(&r.url);
                s.account.notice = Some((t("account.passkey.opened").to_string(), true));
            },
        );
    }

    pub(crate) fn signed_in(&self) -> Option<Stored> {
        self.backend.as_ref().and_then(|b| b.account())
    }

    /// Called when the Account page opens: availability, device list.
    pub fn account_page_opened(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        self.account.vault_ok = b.settings_vault().is_some();
        cx.spawn(async move |this, cx| {
            let available = b.accounts_available().await;
            let devices = if b.account().is_some() { b.account_devices().await.ok() } else { None };
            this.update(cx, |s, cx| {
                s.account.available = Some(available);
                if devices.is_some() {
                    s.account.devices = devices;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn account_sign_in(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        self.account.login = Login::Starting;
        self.account.notice = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let start = b.account_login_start().await;
            let start = match start {
                Ok(s) => s,
                Err(e) => {
                    this.update(cx, |s, cx| {
                        s.account.login = Login::Failed(err_text(&e));
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            let url = start.url.clone();
            this.update(cx, |s, cx| {
                s.account.login = Login::Waiting(start.clone());
                cx.open_url(&url);
                cx.notify();
            })
            .ok();
            let deadline = Instant::now() + Duration::from_secs(start.expires_in_s);
            loop {
                cx.background_executor().timer(POLL_EVERY).await;
                // Cancelled or replaced by another attempt?
                let current = this
                    .update(cx, |s, _| match &s.account.login {
                        Login::Waiting(l) | Login::Full(l) => l.login_id == start.login_id,
                        _ => false,
                    })
                    .unwrap_or(false);
                if !current {
                    return;
                }
                if Instant::now() > deadline {
                    this.update(cx, |s, cx| {
                        s.account.login = Login::Failed(t("account.login.expired").to_string());
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                match b.account_login_poll(&start).await {
                    Ok(LoginPollResponse::Pending) => {}
                    Ok(LoginPollResponse::DeviceLimit) => {
                        this.update(cx, |s, cx| {
                            s.account.login = Login::Full(start.clone());
                            cx.notify();
                        })
                        .ok();
                    }
                    Ok(LoginPollResponse::Done { session }) => {
                        let handle = session.account.handle.clone();
                        this.update(cx, |s, cx| {
                            s.account.login = Login::Idle;
                            s.toast(tf("account.toast.signed_in", &[("handle", &handle)]).to_string());
                            s.account_page_opened(cx);
                            s.sync_now(cx);
                            cx.notify();
                        })
                        .ok();
                        return;
                    }
                    Ok(LoginPollResponse::Expired) => {
                        this.update(cx, |s, cx| {
                            s.account.login = Login::Failed(t("account.login.expired").to_string());
                            cx.notify();
                        })
                        .ok();
                        return;
                    }
                    // Network hiccup: keep polling until the deadline.
                    Err(e) => tracing::info!("login poll: {e:#}"),
                }
            }
        })
        .detach();
    }

    pub fn account_cancel_login(&mut self, cx: &mut Context<Self>) {
        self.account.login = Login::Idle;
        cx.notify();
    }

    /// Run an account call and show its outcome under the actions.
    fn account_call<T: 'static>(
        &mut self,
        cx: &mut Context<Self>,
        call: impl FnOnce(std::sync::Arc<glyph_core::Backend>) -> futures::future::LocalBoxFuture<'static, anyhow::Result<T>> + 'static,
        done: impl FnOnce(&mut Glyph, T, &mut Context<Glyph>) + 'static,
    ) {
        let Some(b) = self.backend.clone() else { return };
        cx.spawn(async move |this, cx| {
            let r = call(b).await;
            this.update(cx, |s, cx| {
                match r {
                    Ok(v) => done(s, v, cx),
                    Err(e) => s.account.notice = Some((err_text(&e), false)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn account_reload_devices(&mut self, cx: &mut Context<Self>) {
        self.account_call(cx, |b| Box::pin(async move { b.account_devices().await }), |s, d, _| s.account.devices = Some(d));
    }

    pub fn account_remove_device(&mut self, id: String, cx: &mut Context<Self>) {
        self.account_call(
            cx,
            move |b| Box::pin(async move { b.account_remove_device(&id).await }),
            |s, _, cx| {
                s.account.notice = Some((t("account.device.removed").to_string(), true));
                s.account_reload_devices(cx);
            },
        );
    }


    pub fn account_rename(&mut self, cx: &mut Context<Self>) {
        let Some(h) = self.account.handle_input.as_ref().map(|i| i.read(cx).value.trim().to_string()).filter(|h| !h.is_empty()) else { return };
        self.account_call(
            cx,
            move |b| Box::pin(async move { b.account_update(AccountPatch { handle: Some(h), ..Default::default() }).await }),
            |s, a, cx| {
                s.account.notice = Some((tf("account.handle.saved", &[("handle", &a.handle)]).to_string(), true));
                if let Some(i) = s.account.handle_input.clone() {
                    i.update(cx, |i, cx| i.set_value("", cx));
                }
            },
        );
    }

    pub fn account_approve(&mut self, cx: &mut Context<Self>) {
        let Some(code) = self.account.code_input.as_ref().map(|i| i.read(cx).value.trim().to_string()).filter(|c| !c.is_empty()) else { return };
        self.account_call(
            cx,
            move |b| Box::pin(async move { b.account_approve(&code).await }),
            |s, r, cx| {
                let msg = if r.device_limit_reached {
                    tf("account.pair.full", &[("name", &r.device_name)])
                } else {
                    tf("account.pair.ok", &[("name", &r.device_name)])
                };
                s.account.notice = Some((msg.to_string(), !r.device_limit_reached));
                if let Some(i) = s.account.code_input.clone() {
                    i.update(cx, |i, cx| i.set_value("", cx));
                }
                s.account_reload_devices(cx);
            },
        );
    }

    pub fn account_recovery(&mut self, cx: &mut Context<Self>) {
        self.account_call(cx, |b| Box::pin(async move { b.account_new_recovery_code().await }), |s, code, _| {
            s.account.recovery_code = Some(code);
            s.account.recovery_visible = false;
        });
    }

    pub fn account_copy_recovery(&mut self, cx: &mut Context<Self>) {
        let Some(code) = self.account.recovery_code.clone() else { return };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(code));
        self.toast(t("account.recovery.copied").to_string());
        cx.notify();
    }

    /// Saves the recovery code as a small text file where the user picks.
    pub fn account_download_recovery(&mut self, cx: &mut Context<Self>) {
        let Some(code) = self.account.recovery_code.clone() else { return };
        let handle = self.signed_in().map(|a| a.account.handle).unwrap_or_default();
        let text = tf("account.recovery.file", &[("handle", &handle), ("code", &code)]).to_string();
        let dir = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(std::path::PathBuf::from)
            .map(|h| if h.join("Downloads").is_dir() { h.join("Downloads") } else { h })
            .unwrap_or_else(glyph_core::paths::data_dir);
        let name = format!("glyph-recovery-code-{handle}.txt");
        let rx = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            let r = cx.background_executor().spawn(async move { std::fs::write(&path, text) }).await;
            this.update(cx, |s, cx| {
                match r {
                    Ok(()) => s.toast(t("account.recovery.saved").to_string()),
                    Err(e) => s.toast_info(tf("sp.transfer.error", &[("e", &e)])),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn account_sign_out(&mut self, cx: &mut Context<Self>) {
        self.account_call(
            cx,
            |b| Box::pin(async move { b.account_sign_out().await }),
            |s, _, _| {
                s.account.devices = None;
                s.account.recovery_code = None;
                s.account.last_sync = None;
                s.toast(t("account.toast.signed_out").to_string());
            },
        );
    }

    pub fn account_delete(&mut self, cx: &mut Context<Self>) {
        let typed = self.account.delete_input.as_ref().map(|i| i.read(cx).value.trim().trim_start_matches('@').to_lowercase()).unwrap_or_default();
        let Some(acc) = self.signed_in() else { return };
        if typed != acc.account.handle {
            self.account.notice = Some((t("account.delete.mismatch").to_string(), false));
            cx.notify();
            return;
        }
        self.account_call(
            cx,
            move |b| Box::pin(async move { b.account_delete(&typed).await }),
            |s, _, _| {
                s.account.devices = None;
                s.account.recovery_code = None;
                s.toast(t("account.toast.deleted").to_string());
            },
        );
    }

    // ------------------------------------------------------------------ sync

    /// Start the background sync loop once the backend is up.
    pub fn start_sync_loop(&mut self, cx: &mut Context<Self>) {
        if self.account.loop_started {
            return;
        }
        self.account.loop_started = true;
        cx.spawn(async move |this, cx| loop {
            if this.update(cx, |s, cx| s.sync_now(cx)).is_err() {
                return;
            }
            // Data saver: sync a quarter as often.
            let saver = this.update(cx, |s, _| s.config.storage.data_saver).unwrap_or(false);
            cx.background_executor().timer(if saver { SYNC_EVERY * 4 } else { SYNC_EVERY }).await;
        })
        .detach();
    }

    /// A library change: sync shortly after the last one.
    pub fn schedule_sync(&mut self, cx: &mut Context<Self>) {
        if self.signed_in().is_none() {
            return;
        }
        self.account.sync_gen += 1;
        let gen = self.account.sync_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SYNC_DEBOUNCE).await;
            this.update(cx, |s, cx| {
                if s.account.sync_gen == gen {
                    s.sync_now(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// One sync round: snapshot here, exchange on the backend, apply here.
    pub fn sync_now(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        if self.account.syncing || !self.library_ok || b.account().is_none() {
            return;
        }
        self.account.syncing = true;
        let snap = glyph_core::sync::snapshot(&self.library, &b.sync_wanted_downloads());
        let settings = glyph_core::settings_sync::items(&self.config, b.settings_vault().as_ref());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = b.sync_exchange(snap, settings).await;
            this.update(cx, |s, cx| {
                s.account.syncing = false;
                match r {
                    Ok(round) => {
                        // Songs of auto-download playlists before the round,
                        // to find the ones another device added.
                        let auto_before: Vec<(String, std::collections::HashSet<String>)> = s
                            .library
                            .playlists
                            .iter()
                            .filter(|p| p.auto_download)
                            .map(|p| (p.id.clone(), p.tracks.iter().map(glyph_core::dedupe::track_key).collect()))
                            .collect();
                        let applied = b.sync_apply(&mut s.library, &round);
                        let mut cfg = s.config.clone();
                        let st = b.sync_apply_settings(&mut cfg, &round);
                        if st.changed {
                            s.adopt_synced_config(cfg, st.providers_changed, cx);
                        }
                        s.account.vault_ok = b.settings_vault().is_some();
                        if applied.library_changed {
                            let lib = s.library.clone();
                            cx.background_executor()
                                .spawn(async move {
                                    if let Err(e) = guard_res(|| lib.save()) {
                                        tracing::warn!("library save after sync: {e}");
                                    }
                                })
                                .detach();
                        }
                        if !applied.download.is_empty() {
                            s.download_tracks(applied.download, cx);
                        }
                        if !applied.remove_downloads.is_empty() {
                            s.remove_downloads(applied.remove_downloads, cx);
                        }
                        for (id, before) in auto_before {
                            let added: Vec<_> = s
                                .library
                                .playlist(&id)
                                .map(|p| p.tracks.iter().filter(|t| !before.contains(&glyph_core::dedupe::track_key(t))).cloned().collect())
                                .unwrap_or_default();
                            s.auto_download_added(&id, added, cx);
                        }
                        s.account.last_sync = Some(Instant::now());
                        s.account.sync_error = None;
                    }
                    Err(e) => {
                        if is_signed_out(&e) {
                            s.account.devices = None;
                        }
                        tracing::info!("sync: {e:#}");
                        s.account.sync_error = Some(err_text(&e));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

// ------------------------------------------------------------------ page

fn ago(i: Instant) -> SharedString {
    let s = i.elapsed().as_secs();
    if s < 60 {
        t("account.sync.just_now")
    } else {
        tf("account.sync.min_ago", &[("n", &(s / 60))])
    }
}

fn when_ms(ms: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let d = now.saturating_sub(ms) / 1000;
    match d {
        0..=119 => t("account.sync.just_now").to_string(),
        120..=7199 => tf("account.sync.min_ago", &[("n", &(d / 60))]).to_string(),
        7200..=172_799 => tf("account.hours_ago", &[("n", &(d / 3600))]).to_string(),
        _ => tf("account.days_ago", &[("n", &(d / 86_400))]).to_string(),
    }
}

fn danger_button(id: impl Into<gpui::ElementId>, label: SharedString) -> gpui::Stateful<Div> {
    button(id, label).text_color(theme::danger())
}

fn code_box(code: &str) -> Div {
    div()
        .my(px(8.0))
        .py(px(14.0))
        .flex()
        .justify_center()
        .rounded(px(8.0))
        .border_1()
        .border_dashed()
        .border_color(white(0.25))
        .font_family(theme::MONO)
        .text_size(px(24.0))
        .text_color(gpui::white())
        .child(code.to_string())
}

pub fn page(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    app.account_inputs(cx);
    let col = div().flex().flex_col();
    if app.backend.is_none() || app.account.available == Some(false) {
        return col.child(div().py(px(14.0)).text_size(px(13.0)).text_color(white(0.6)).child(t("account.unavailable")));
    }
    let notice = app.account.notice.clone().map(|(m, ok)| {
        div().pt(px(8.0)).text_size(px(12.5)).text_color(if ok { theme::ok() } else { theme::warn() }).child(m)
    });
    match app.signed_in() {
        None => signed_out(app, col, cx).children(notice),
        Some(acc) => signed_in(app, acc, col, cx).children(notice),
    }
}

fn field(input: Entity<TextInput>) -> Div {
    input_box(input).w_full().h(px(38.0))
}

fn signed_out(app: &mut Glyph, col: Div, cx: &mut Context<Glyph>) -> Div {
    // Centred in the page width, a little below the title.
    let centre = div().w_full().pt(px(24.0)).flex().flex_col().items_center();
    if let Login::Starting | Login::Waiting(_) | Login::Full(_) = &app.account.login {
        return col.child(centre.child(passkey_waiting(app, cx)));
    }
    let public = app.config.effective_server_url().trim_end_matches('/') == glyph_core::config::DEFAULT_SERVER_URL;

    if public && !app.account.in_app {
        // Just the two ways in, centred in the page.
        let failed = match &app.account.login {
            Login::Failed(m) => Some(div().text_center().text_size(px(12.5)).text_color(theme::warn()).child(m.clone())),
            _ => None,
        };
        let buttons = div()
            .w(px(320.0))
            .flex()
            .flex_col()
            .gap(px(10.0))
            .children(failed)
            .child(
                primary_button("acct-browser", t("account.browser.button"))
                    .justify_center()
                    .h(px(40.0))
                    .child(icon("external-link", 14.0, gpui::black().opacity(0.6)))
                    .on_click(cx.listener(|this, _, _, cx| this.account_sign_in(cx))),
            )
            .child(
                button("acct-inapp", t("account.browser.in_app"))
                    .justify_center()
                    .h(px(40.0))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.account.in_app = true;
                        this.account.login = Login::Idle;
                        cx.notify();
                    })),
            );
        return col.child(div().w_full().h(px(400.0)).flex().flex_col().items_center().justify_center().child(buttons));
    }
    let a = &app.account;
    let register = a.register;
    let mode = |r: bool| -> Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)> { Rc::new(move |s, cx| s.account_set_mode(r, cx)) };
    let failed = match &a.login {
        Login::Failed(m) => Some(div().text_size(px(12.5)).text_color(theme::warn()).child(m.clone())),
        _ => None,
    };
    let submit_label = if a.busy {
        SharedString::from(format!("{} {}", app.spinner(), t("account.form.working")))
    } else if register {
        t("account.form.create")
    } else {
        t("account.signin.button")
    };
    let form = div()
        .w(px(360.0))
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(segmented(vec![(t("account.signin.button"), !register, mode(false)), (t("account.form.new"), register, mode(true))], cx, "acct-mode"))
        .child(field(a.user_input.clone().unwrap()))
        .child(field(a.pw_input.clone().unwrap()))
        .when(register, |d| d.child(field(a.pw2_input.clone().unwrap())))
        .when(register, |d| d.child(div().text_size(px(12.0)).text_color(white(0.5)).child(t("account.form.rules"))))
        .children(failed)
        .child(
            primary_button("acct-submit", submit_label)
                .justify_center()
                .h(px(38.0))
                .on_click(cx.listener(|this, _, _, cx| this.account_password_submit(cx))),
        );
    let or = div()
        .w(px(360.0))
        .py(px(14.0))
        .flex()
        .items_center()
        .gap(px(10.0))
        .child(div().flex_1().h(px(1.0)).bg(white(0.08)))
        .child(div().text_size(px(12.0)).text_color(white(0.45)).child(t("account.form.or")))
        .child(div().flex_1().h(px(1.0)).bg(white(0.08)));
    let passkey = div()
        .w(px(360.0))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(
            button("acct-signin", t("account.passkey.button"))
                .justify_center()
                .h(px(38.0))
                .child(icon("external-link", 14.0, white(0.6)))
                .on_click(cx.listener(|this, _, _, cx| this.account_sign_in(cx))),
        )
        .child(div().text_center().text_size(px(12.0)).text_color(white(0.5)).child(t("account.passkey.desc")));
    let back = public.then(|| sign_in_switch("acct-usebrowser", t("account.browser.back"), false, cx));
    col.child(centre.child(form).child(or).child(passkey).children(back))
}

/// Plain text link (no button look) between browser and in-app sign-in.
fn sign_in_switch(id: &'static str, label: SharedString, in_app: bool, cx: &mut Context<Glyph>) -> impl IntoElement {
    div()
        .id(id)
        .pt(px(14.0))
        .cursor_pointer()
        .text_size(px(12.5))
        .text_color(white(0.5))
        .hover(|d| d.text_color(white(0.9)))
        .child(label)
        .on_click(cx.listener(move |this, _, _, cx| {
            this.account.in_app = in_app;
            this.account.login = Login::Idle;
            cx.notify();
        }))
}

fn passkey_waiting(app: &mut Glyph, cx: &mut Context<Glyph>) -> AnyElement {
    match app.account.login.clone() {
        Login::Waiting(l) | Login::Full(l) => {
            let full = matches!(app.account.login, Login::Full(_));
            let url = l.url.clone();
            div()
                .w(px(420.0))
                .flex()
                .flex_col()
                .child(div().text_size(px(13.0)).text_color(white(0.75)).child(t("account.login.waiting")))
                .child(code_box(&l.user_code))
                .child(div().text_size(px(12.5)).text_color(white(0.55)).child(t("account.login.pair_hint")))
                .when(full, |d| d.child(div().pt(px(8.0)).text_size(px(12.5)).text_color(theme::warn()).child(t("account.login.full"))))
                .child(
                    div()
                        .pt(px(12.0))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(spinner_line(app.spinner(), t("account.login.polling")))
                        .child(div().flex_1())
                        .child(button("acct-open", t("account.login.reopen")).on_click(move |_, _, cx| cx.open_url(&url)))
                        .child(button("acct-cancel", t("account.login.cancel")).on_click(cx.listener(|this, _, _, cx| this.account_cancel_login(cx)))),
                )
                .into_any_element()
        }
        _ => spinner_line(app.spinner(), t("account.login.starting")).into_any_element(),
    }
}

/// Settings sync row: API keys are end-to-end encrypted with a key from the password.
fn keys_row(app: &Glyph, acc: &Stored, cx: &mut Context<Glyph>) -> AnyElement {
    let label = t("account.keys");
    let dim = |s: SharedString| div().text_size(px(13.0)).text_color(white(0.55)).child(s);
    if !acc.account.has_password {
        return row(label, Some(t("account.keys.need_password")), dim(t("account.keys.off"))).into_any_element();
    }
    if app.account.vault_ok {
        return row(label, Some(t("account.keys.desc")), div().text_size(px(13.0)).text_color(white(0.85)).child(t("account.keys.on"))).into_any_element();
    }
    let input = app.account.unlock_input.clone().unwrap();
    row(
        label,
        Some(t("account.keys.unlock_desc")),
        div()
            .flex()
            .gap(px(8.0))
            .child(input_box(input).w(px(180.0)))
            .child(button("acct-unlock", t("account.keys.unlock")).on_click(cx.listener(|this, _, _, cx| this.account_unlock_keys(cx)))),
    )
    .into_any_element()
}

fn signed_in(app: &mut Glyph, acc: Stored, col: Div, cx: &mut Context<Glyph>) -> Div {
    let a = &app.account;
    let status: SharedString = if a.syncing {
        format!("{} {}", app.spinner(), t("account.sync.running")).into()
    } else if let Some(e) = &a.sync_error {
        e.clone().into()
    } else if let Some(at) = a.last_sync {
        tf("account.sync.done", &[("when", &ago(at))])
    } else {
        t("account.sync.never")
    };
    let limit = a.devices.as_ref().map_or(acc.account.device_limit, |d| d.device_limit);

    let (handle_in, code_in, delete_in) = (a.handle_input.clone().unwrap(), a.code_input.clone().unwrap(), a.delete_input.clone().unwrap());

    let head = rows(vec![
        row(
            format!("@{}", acc.account.handle),
            Some(status),
            div()
                .flex()
                .gap(px(8.0))
                .child(button("acct-sync", t("account.sync.now")).on_click(cx.listener(|this, _, _, cx| this.sync_now(cx))))
                .child(button("acct-signout", t("account.signout")).on_click(cx.listener(|this, _, _, cx| this.account_sign_out(cx)))),
        ),
        row(
            t("account.handle"),
            Some(t("account.handle.desc")),
            div()
                .flex()
                .gap(px(8.0))
                .child(input_box(handle_in).w(px(200.0)))
                .child(button("acct-rename", t("account.handle.save")).on_click(cx.listener(|this, _, _, cx| this.account_rename(cx)))),
        ),
        row(
            t("account.limit"),
            Some(t("account.limit.desc")),
            div().text_size(px(13.0)).text_color(white(0.85)).child(limit.to_string()),
        ),
        keys_row(app, &acc, cx),
    ]);

    let devices: Vec<AnyElement> = match &a.devices {
        None => vec![spinner_line(app.spinner(), t("account.devices.loading")).into_any_element()],
        Some(d) => d
            .devices
            .iter()
            .enumerate()
            .map(|(i, dev)| {
                let id = dev.id.clone();
                let location = dev.approximate_location.as_deref().map(|s| format!("≈ {s}")).unwrap_or_else(|| t("account.devices.location_unknown").to_string());
                let desc: SharedString = format!("{} · {} · {}", dev.platform, when_ms(dev.last_seen), location).into();
                let control = if dev.current {
                    div().flex().items_center().gap(px(12.0))
                        .child(div().text_size(px(12.5)).text_color(white(0.5)).child(t("account.devices.this")))
                        .child(danger_button(("acct-dev-self", i), t("account.devices.signout")).on_click(cx.listener(|this, _, _, cx| this.account_sign_out(cx))))
                        .into_any_element()
                } else {
                    danger_button(("acct-dev-rm", i), t("account.devices.signout"))
                        .on_click(cx.listener(move |this, _, _, cx| this.account_remove_device(id.clone(), cx)))
                        .into_any_element()
                };
                row(dev.name.clone(), Some(desc), control)
            })
            .collect(),
    };
    let count = a.devices.as_ref().map_or(0, |d| d.devices.len());

    let pair = rows(vec![row(
        t("account.pair"),
        Some(t("account.pair.desc")),
        div()
            .flex()
            .gap(px(8.0))
            .child(input_box(code_in).w(px(140.0)))
            .child(button("acct-approve", t("account.pair.approve")).on_click(cx.listener(|this, _, _, cx| this.account_approve(cx)))),
    )]);

    let (np, np2) = (a.newpw_input.clone().unwrap(), a.newpw2_input.clone().unwrap());
    let recovery = rows(vec![
        row(
            if acc.account.has_password { t("account.password.change") } else { t("account.password.set") },
            Some(t("account.password.desc")),
            div()
                .flex()
                .gap(px(8.0))
                .child(input_box(np).w(px(150.0)))
                .child(input_box(np2).w(px(150.0)))
                .child(button("acct-pw", t("account.handle.save")).on_click(cx.listener(|this, _, _, cx| this.account_change_password(cx)))),
        ),
        row(
            t("account.passkey.add"),
            Some(t("account.passkey.add.desc")),
            button("acct-link", t("account.passkey.add.button"))
                .child(icon("external-link", 14.0, white(0.6)))
                .on_click(cx.listener(|this, _, _, cx| this.account_link_passkey(cx))),
        ),
        row(
            t("account.recovery"),
            Some(t("account.recovery.desc")),
            button("acct-rc", t("account.recovery.new")).on_click(cx.listener(|this, _, _, cx| this.account_recovery(cx))),
        ),
    ]);
    let rc_shown = a.recovery_code.clone().map(|c| recovery_box(&c, a.recovery_visible, cx));

    let delete = rows(vec![row(
        t("account.delete"),
        Some(tf("account.delete.desc", &[("handle", &acc.account.handle)])),
        div()
            .flex()
            .gap(px(8.0))
            .child(input_box(delete_in).w(px(160.0)))
            .child(danger_button("acct-delete", t("account.delete.button")).on_click(cx.listener(|this, _, _, cx| this.account_delete(cx)))),
    )]);

    // Sub-tabs (see `settings::subtabs`): overview, devices, security.
    match app.settings_sub {
        1 => col
            .child(section_label(tf("account.devices", &[("n", &count), ("max", &limit)])))
            .child(rows(devices))
            .child(section_label(t("account.pair.section")))
            .child(pair),
        2 => col
            .child(recovery)
            .children(rc_shown)
            .child(section_label(t("account.danger")))
            .child(delete),
        // A code from signing up shows right here too.
        _ => col
            .child(head)
            .children(rc_shown)
            .child(div().pt(px(14.0)).text_size(px(12.0)).text_color(white(0.4)).font_weight(FontWeight::NORMAL).child(t("account.privacy"))),
    }
}

/// A fresh recovery code: dots until clicked, then the code; copy and save.
fn recovery_box(code: &str, visible: bool, cx: &mut Context<Glyph>) -> Div {
    let shown: String = if visible {
        code.to_string()
    } else {
        code.chars().map(|c| if c == '-' { '-' } else { '•' }).collect()
    };
    let code_line = div()
        .id("acct-rc-code")
        .my(px(8.0))
        .py(px(14.0))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(4.0))
        .rounded(px(8.0))
        .border_1()
        .border_dashed()
        .border_color(white(0.25))
        .when(!visible, |d| d.cursor_pointer().hover(|s| s.bg(white(0.03))))
        .on_click(cx.listener(|this, _, _, cx| {
            this.account.recovery_visible = true;
            cx.notify();
        }))
        .child(div().font_family(theme::MONO).text_size(px(22.0)).text_color(if visible { gpui::white() } else { white(0.55) }).child(shown))
        .when(!visible, |d| d.child(div().text_size(px(12.0)).text_color(white(0.5)).child(t("account.recovery.reveal"))));
    div()
        .child(code_line)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(div().flex_1().text_size(px(12.5)).text_color(theme::warn()).child(t("account.recovery.once")))
                .child(
                    button("acct-rc-toggle", if visible { t("account.recovery.hide") } else { t("account.recovery.show") })
                        .child(icon(if visible { "eye-off" } else { "eye" }, 14.0, white(0.6)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.account.recovery_visible = !visible;
                            cx.notify();
                        })),
                )
                .child(
                    button("acct-rc-copy", t("account.recovery.copy"))
                        .child(icon("copy", 14.0, white(0.6)))
                        .on_click(cx.listener(|this, _, _, cx| this.account_copy_recovery(cx))),
                )
                .child(
                    button("acct-rc-save", t("account.recovery.download"))
                        .child(icon("download", 14.0, white(0.6)))
                        .on_click(cx.listener(|this, _, _, cx| this.account_download_recovery(cx))),
                ),
        )
}
