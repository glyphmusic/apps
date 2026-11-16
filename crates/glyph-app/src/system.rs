//! Settings backup, equalizer sharing, autostart, MCP info, yt-dlp,
//! diagnostics, backend retry and updates (from `src/app/settings.rs`,
//! `src/ui/transfer.rs`, `src/ui/ytdlp.rs`, `src/app/boot.rs`).

use crate::controller::Core;
use crate::settings::{apply_to_player, eq_gains};
use glyph_core::eq::{self, EqPreset};
use glyph_core::settings_share;
use glyph_core::updater::UpdateStatus;
use glyph_core::Config;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use glyph_protocol::{ProviderId, Track};

type Res<T = ()> = Result<T, String>;
type C<'a> = &'a Arc<Core>;

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

/// Called once at start-up from `glyph_app::setup`.
pub fn setup(core: &Arc<Core>) {
    crate::mcp_tools::start(core);
    // Each time a (new) backend is ready: the update check of the start.
    let core = core.clone();
    crate::rt::spawn(async move {
        let mut last = 0usize;
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let Some(b) = core.backend() else { continue };
            let ptr = Arc::as_ptr(&b) as usize;
            if ptr == last {
                continue;
            }
            let first = last == 0;
            last = ptr;
            let on = core.st().config.updates.check_on_start;
            if first && on {
                let c = core.clone();
                crate::rt::spawn(async move {
                    let _ = run_check(&c, false).await;
                });
            }
        }
    });
}

// ------------------------------------------------------- backup & share

/// Settings before the last import ("Undo").
fn undo() -> MutexGuard<'static, Option<Config>> {
    static U: Mutex<Option<Config>> = Mutex::new(None);
    U.lock().unwrap_or_else(|e| e.into_inner())
}

/// Takes `cfg` as the new settings (save, apply to the player and backend).
/// Changed sources or keys rebuild the backend.
async fn adopt(core: &Arc<Core>, cfg: Config, providers_changed: bool) {
    core.st().config = cfg.clone();
    apply_to_player(core, &cfg);
    core.config_changed();
    if providers_changed {
        core.set_backend_state(json!({ "kind": "connecting" }));
        core.start_backend();
    } else if let Some(b) = core.backend() {
        if let Err(e) = b.set_config(cfg).await {
            tracing::warn!(error = %e, "settings: backend refused");
        }
    }
}

pub fn settings_code(core: C) -> String {
    settings_share::encode(&core.st().config)
}

pub fn settings_file(core: C) -> Value {
    json!({ "name": settings_share::FILE_NAME, "text": settings_share::to_json(&core.st().config) })
}

/// Applies a settings code or file content; the old settings stay for undo.
pub async fn settings_import(core: C<'_>, text: String) -> Res<Value> {
    let shared = settings_share::decode(&text).map_err(err)?;
    let before = core.st().config.clone();
    let mut cfg = before.clone();
    let r = shared.apply(&mut cfg);
    if r.changed {
        *undo() = Some(before);
        adopt(core, cfg, r.providers_changed).await;
    }
    Ok(json!({ "changed": r.changed }))
}

pub async fn settings_undo(core: C<'_>) -> Res {
    let Some(old) = undo().take() else { return Ok(()) };
    let providers_changed = old.providers != core.st().config.providers;
    adopt(core, old, providers_changed).await;
    Ok(())
}

// ----------------------------------------------------------- equalizer

fn preset(name: &str, gains: &[f32]) -> EqPreset {
    EqPreset::new(name, eq_gains(gains))
}

/// Applies at once, does not save.
pub fn set_eq_live(core: C, enabled: bool, gains: Vec<f32>) {
    if let Some(p) = &core.player {
        p.set_equalizer(enabled, eq_gains(&gains));
    }
}

pub fn eq_code(name: String, gains: Vec<f32>) -> String {
    eq::encode(&preset(&name, &gains))
}

pub fn eq_file(name: String, gains: Vec<f32>) -> Value {
    let p = preset(&name, &gains);
    let file = format!("{}.{}", p.name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "-"), eq::FILE_EXT);
    json!({ "name": file, "text": eq::to_file(&p) })
}

/// A share code or `.glyph-eq` file text; `null` when it is neither.
pub fn eq_import(text: String) -> Res<Option<Value>> {
    if !eq::is_code(&text) && !text.trim().starts_with('{') {
        return Ok(None);
    }
    let p = eq::decode(&text).map_err(err)?;
    Ok(Some(json!({ "name": p.name, "gains": p.gains() })))
}

/// Saves the current curve as an own preset (same name: replaced) and selects it.
pub fn eq_preset_save(core: C, name: String) {
    let cfg = {
        let mut st = core.st();
        let gains = eq_gains(&st.config.audio.eq_gains);
        let p = EqPreset::new(&name, gains);
        if p.name.is_empty() {
            return;
        }
        let a = &mut st.config.audio;
        match a.eq_user_presets.iter_mut().find(|e| e.name.eq_ignore_ascii_case(&p.name)) {
            Some(e) => *e = p.clone(),
            None => a.eq_user_presets.push(p.clone()),
        }
        a.eq_enabled = true;
        a.eq_preset = format!("user:{}", p.name);
        a.eq_gains = p.gains().to_vec();
        st.config.clone()
    };
    apply_to_player(&core, &cfg);
    core.config_changed();
}

// -------------------------------------------------------------- system

/// Start with the computer on / off. Rejects with the error text; the UI
/// writes the setting itself.
pub fn set_autostart(on: bool) -> Res {
    crate::host::get().set_autostart(on).map_err(|e| {
        tracing::warn!("autostart: {e}");
        e
    })
}

/// Version, OS and setup summary for bug reports (no keys, no personal data).
pub fn diagnostics(core: C) -> String {
    let backend = {
        let v = core.backend_state.lock().unwrap_or_else(|e| e.into_inner()).clone();
        match v["kind"].as_str() {
            Some("ready") => format!("ready ({})", v["mode"].as_str().unwrap_or("")),
            Some(k) => format!("{k} {}", v["reason"].as_str().unwrap_or("")).trim().to_string(),
            None => String::new(),
        }
    };
    let st = core.st();
    let lang = if st.config.language.is_empty() { "en" } else { st.config.language.as_str() };
    format!(
        "Glyph {}\nOS: {} {}\nTarget: {}\nProtocol: v{}\nBackend: {}\nProviders: {:?}\nLanguage: {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        glyph_core::updater::current_target(),
        glyph_protocol::PROTOCOL_VERSION,
        backend,
        st.config.providers.enabled,
        lang,
    )
}

/// Builds the backend again with the current settings.
pub fn retry_backend(core: C) {
    core.set_backend_state(json!({ "kind": "connecting" }));
    core.start_backend();
}

// ----------------------------------------------------------------- MCP

pub fn mcp_info(core: C) -> Value {
    crate::mcp_tools::info(core)
}

pub fn mcp_rotate_token(core: C) {
    crate::mcp_tools::rotate_token(core);
}

pub async fn mcp_server_token(core: C<'_>) -> Res {
    crate::mcp_tools::request_server_token(core, None).await;
    Ok(())
}

// -------------------------------------------------------------- yt-dlp

fn ytdlp_json(s: &glyph_core::ytdlp::YtDlpStatus) -> Value {
    json!({
        "path": s.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
        "version": s.version, "js_runtime": s.js_runtime, "can_install": s.can_install, "managed": s.managed,
        "update_available": s.update_available, "latest_version": s.latest_version, "update_error": s.update_error,
    })
}

/// The last yt-dlp status and when it was read (kept 15 minutes, like the
/// GPUI settings page; `force` reads again).
fn ytdlp_cache() -> MutexGuard<'static, Option<(Instant, Value)>> {
    static C: Mutex<Option<(Instant, Value)>> = Mutex::new(None);
    C.lock().unwrap_or_else(|e| e.into_inner())
}

pub async fn ytdlp_status(core: C<'_>, force: Option<bool>) -> Res<Value> {
    if !force.unwrap_or(false) {
        if let Some((at, v)) = &*ytdlp_cache() {
            if at.elapsed() < Duration::from_secs(900) {
                return Ok(v.clone());
            }
        }
    }
    let b = core.backend_or_err()?;
    let v = ytdlp_json(&b.ytdlp_status().await.map_err(err)?);
    *ytdlp_cache() = Some((Instant::now(), v.clone()));
    Ok(v)
}

pub async fn ytdlp_install(core: C<'_>) -> Res<Value> {
    let b = core.backend_or_err()?;
    let v = ytdlp_json(&b.install_ytdlp().await.map_err(err)?);
    *ytdlp_cache() = Some((Instant::now(), v.clone()));
    Ok(v)
}

/// The pointer rested on a song row: look the song up now (YouTube songs
/// also get a local yt-dlp going), so a click plays at once
/// (`prefetch_on_hover` in `src/ui/ytdlp.rs`).
pub fn prefetch_hover(core: C, track: Track) {
    let Some(b) = core.backend() else { return };
    let key = glyph_core::dedupe::track_key(&track);
    if core.st().current.as_ref().is_some_and(|c| glyph_core::dedupe::track_key(c) == key) {
        return;
    }
    let youtube = track.source == ProviderId::YouTube || track.external_ids.contains_key(&ProviderId::YouTube);
    if youtube && b.local_youtube() {
        b.prefetch(vec![track]);
    } else {
        b.warm(vec![track]);
    }
}

// ------------------------------------------------------------- updates

#[derive(Default)]
struct Upd {
    checking: bool,
    installing: Option<(u64, u64)>,
    installed: bool,
    error: Option<String>,
    status: Option<UpdateStatus>,
}

fn upd() -> MutexGuard<'static, Upd> {
    static U: std::sync::OnceLock<Mutex<Upd>> = std::sync::OnceLock::new();
    U.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

fn status_json(s: &Option<UpdateStatus>) -> Value {
    match s {
        None => json!({ "kind": "none" }),
        Some(UpdateStatus::UpToDate { current }) => json!({ "kind": "up_to_date", "current": current }),
        Some(UpdateStatus::Skipped) => json!({ "kind": "skipped" }),
        Some(UpdateStatus::Available(i)) => json!({
            "kind": "available", "version": i.version, "can_self_install": i.can_self_install,
            "html_url": i.html_url, "release_package": glyph_core::updater::is_release_package(),
        }),
    }
}

/// Checks for a newer version. `manual`: the "Check" button (never skipped,
/// never installs by itself); else the start-up check.
pub async fn run_check(core: &Arc<Core>, manual: bool) -> Res<Value> {
    let b = core.backend_or_err()?;
    {
        let mut u = upd();
        u.checking = true;
        u.error = None;
    }
    let r = if manual { b.check_for_update().await } else { b.check_for_update_auto().await };
    let mut auto_install = None;
    let out = {
        let mut u = upd();
        u.checking = false;
        match r {
            Ok(UpdateStatus::Skipped) if !manual => {}
            Ok(st) => {
                if let UpdateStatus::Available(i) = &st {
                    core.emit("update", json!({ "version": i.version, "notes": i.notes }));
                    if !manual && i.can_self_install && core.st().config.updates.auto_install {
                        auto_install = Some(i.clone());
                    }
                }
                u.status = Some(st);
            }
            Err(e) => u.error = Some(format!("{e:#}")),
        }
        match &u.error {
            Some(e) if manual => Err(e.clone()),
            _ => Ok(status_json(&u.status)),
        }
    };
    if auto_install.is_some() {
        let c = core.clone();
        crate::rt::spawn(async move {
            let _ = install(&c).await;
        });
    }
    out
}

/// Downloads, verifies and installs the update found by the last check.
pub async fn install(core: &Arc<Core>) -> Res {
    let b = core.backend_or_err()?;
    let info = match &upd().status {
        Some(UpdateStatus::Available(i)) => i.clone(),
        _ => return Err("no update available".into()),
    };
    upd().installing = Some((0, 0));
    let (tx, mut rx) = glyph_core::flume_like::channel::<(u64, u64)>();
    crate::rt::spawn(async move {
        while let Some(p) = rx.recv().await {
            upd().installing = Some(p);
        }
    });
    let r = b.install_update(info, tx).await;
    let mut u = upd();
    u.installing = None;
    match r {
        Ok(()) => {
            u.installed = true;
            Ok(())
        }
        Err(e) => {
            u.error = Some(format!("{e:#}"));
            Err(format!("{e:#}"))
        }
    }
}

pub fn update_state() -> Value {
    let u = upd();
    json!({ "checking": u.checking, "installing": u.installing, "installed": u.installed, "error": u.error, "status": status_json(&u.status) })
}

pub async fn update_check(core: C<'_>, manual: Option<bool>) -> Res<Value> {
    run_check(core, manual.unwrap_or(true)).await
}

pub async fn update_install(core: C<'_>) -> Res {
    install(core).await
}

pub fn update_restart(core: C) {
    core.save_if_dirty();
    core.host.restart();
}
