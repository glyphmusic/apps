//! Plugins: catalog from GitHub (never through glyph-server), install,
//! enable, the settings rows they add, and their code at work. From
//! `src/app/plugins.rs`, `src/app/plugin_api.rs` and `src/ui/settings/plugin_ui.rs`.
//! The UI (plugins-state.svelte.ts) keeps the lists; this file keeps what
//! is on disk and what runs.

use crate::controller::Core;
use glyph_core::plugin_host::{self, HostCall, Running};
use glyph_core::plugins::{self, Catalog, Installed, Invalid, Listing, SettingItem};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

type Res<T = ()> = Result<T, String>;
type C<'a> = &'a Arc<Core>;

/// Answers the plugin functions that map to an MCP tool (`player.*`,
/// `queue.*`, `search`, `library.*`, `lyrics.get`, `stats.get`). Gets the
/// tool name and arguments, returns the MCP result (`content`, `isError`,
/// `structuredContent`). Set it with [`set_mcp_handler`].
pub type McpHandler = fn(&Arc<Core>, &str, &Value) -> Option<Value>;
static MCP: OnceLock<McpHandler> = OnceLock::new();

/// For `mcp_tools.rs`: lets plugins use the same tools as AI assistants.
pub fn set_mcp_handler(f: McpHandler) {
    let _ = MCP.set(f);
}

/// Plugin functions answered by an MCP tool of the same meaning.
const MCP_FUNCS: &[(&str, &str)] = &[
    ("player.state", "now_playing"),
    ("player.play", "play"),
    ("player.pause", "pause"),
    ("player.toggle", "toggle_play"),
    ("player.next", "next"),
    ("player.previous", "previous"),
    ("player.seek", "seek"),
    ("player.volume", "set_volume"),
    ("player.shuffle", "set_shuffle"),
    ("player.repeat", "set_repeat"),
    ("player.play_song", "play_song"),
    ("player.play_artist", "play_artist"),
    ("queue.get", "get_queue"),
    ("queue.add", "queue_add"),
    ("search", "search"),
    ("library.playlists", "list_playlists"),
    ("library.playlist", "get_playlist"),
    ("library.play_playlist", "play_playlist"),
    ("library.create_playlist", "create_playlist"),
    ("library.rename_playlist", "rename_playlist"),
    ("library.delete_playlist", "delete_playlist"),
    ("library.add_to_playlist", "add_to_playlist"),
    ("library.remove_from_playlist", "remove_from_playlist"),
    ("library.like", "like"),
    ("library.unlike", "unlike"),
    ("lyrics.get", "get_lyrics"),
    ("stats.get", "listening_stats"),
];

struct Rt {
    installed: Vec<Installed>,
    catalog: Option<Catalog>,
    /// Running plugins with the version and install time they started at
    /// (a change restarts them).
    running: HashMap<String, (Running, String, u64)>,
    values: HashMap<String, Map<String, Value>>,
    /// Changes plugins made to their rows (`ui.update`), per plugin and item.
    patches: HashMap<String, HashMap<String, Map<String, Value>>>,
    stopped: HashMap<String, String>,
    missing_credentials: HashMap<String, Vec<String>>,
    configured: HashSet<(String, String)>,
    server_url: String,
    budget: glyph_core::plugin_policy::CallBudget,
    last_track: Option<String>,
    last_playing: bool,
    /// Lyrics plugins published: plugin id -> (song key as the UI writes it, lyrics).
    lyrics: HashMap<String, (String, glyph_core::lyrics::Lyrics)>,
}

pub struct PluginState {
    rt: Mutex<Rt>,
    tx: flume::Sender<HostCall>,
    rx: flume::Receiver<HostCall>,
}

impl PluginState {
    fn lock(&self) -> MutexGuard<'_, Rt> {
        self.rt.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Returns the accent colour declared by an installed plugin manifest, if any.
pub(crate) fn theme_accent(st: &PluginState, plugin_id: &str) -> Option<u32> {
    st.lock()
        .installed
        .iter()
        .find(|p| p.manifest.id == plugin_id)
        .and_then(|p| p.manifest.theme_colors().map(|c| c.accent))
}

/// Returns `plugin:<id>` theme IDs provided by installed theme plugins.
pub(crate) fn installed_theme_ids(st: &PluginState) -> Vec<String> {
    st.lock()
        .installed
        .iter()
        .filter(|p| p.manifest.theme_colors().is_some())
        .map(|p| format!("plugin:{}", p.manifest.id))
        .collect()
}

/// Called once at start-up from `glyph_app::setup`.
pub fn setup(core: &Arc<Core>) {
    let (tx, rx) = flume::bounded(128);
    let st = Arc::new(PluginState {
        rt: Mutex::new(Rt {
            installed: Vec::new(),
            catalog: None,
            running: HashMap::new(),
            values: HashMap::new(),
            patches: HashMap::new(),
            stopped: HashMap::new(),
            missing_credentials: HashMap::new(),
            configured: HashSet::new(),
            server_url: String::new(),
            budget: Default::default(),
            last_track: None,
            last_playing: false,
            lyrics: HashMap::new(),
        }),
        tx,
        rx,
    });
    core.set_plugins(st.clone());
    let core = core.clone();
    std::thread::Builder::new()
        .name("glyph-plugins".into())
        .spawn(move || {
            load(&st);
            sync_code(&core, &st);
            crate::theme_icon::sync_host_icon(&core);
            loop {
                std::thread::sleep(Duration::from_millis(100));
                poll(&core, &st);
            }
        })
        .expect("spawn plugins thread");
}

// ---- state on disk ----

fn disabled(core: &Core) -> Vec<String> {
    core.st().config.plugins.disabled.clone()
}

fn may_run(p: &Installed, disabled: &[String]) -> bool {
    p.manifest.main.is_some() && !disabled.contains(&p.manifest.id) && (p.origin.checked || p.origin.is_local())
}

/// Reads the installed plugins (and the cached catalog) from disk.
fn load(st: &PluginState) {
    let mut rt = st.lock();
    rt.installed = plugins::installed();
    if rt.catalog.is_none() {
        rt.catalog = plugins::cached_catalog();
    }
}

/// Starts plugins that should run and stops the others; an updated plugin
/// restarts. Also loads every plugin's settings values.
fn sync_code(core: &Arc<Core>, st: &PluginState) {
    // Which plugins run may have changed: lyrics of a stopped one go away.
    core.emit("plugin_lyrics_reset", ());
    let (server_url, dis, lang) = {
        let s = core.st();
        (s.config.effective_server_url().to_string(), s.config.plugins.disabled.clone(), s.config.resolved_language())
    };
    let song = core.st().current.clone();
    let song = song.map(|t| track_json(core, &t));
    let playing = core.player_state().is_some_and(|s| s.status == glyph_audio::Status::Playing);
    let mut rt = st.lock();
    if rt.server_url != server_url {
        rt.running.clear();
        rt.server_url = server_url.clone();
    }
    rt.missing_credentials.clear();
    rt.configured.clear();
    let installed = rt.installed.clone();
    for p in &installed {
        let mut values = plugins::load_values(&p.manifest.id);
        let mut changed = false;
        let mut missing = Vec::new();
        for item in p.manifest.settings.iter().flat_map(|s| &s.items).filter(|i| i.secret) {
            // Old plaintext keys have no trusted destination: removed.
            if values.remove(&item.id).is_some() {
                changed = true;
            }
            if glyph_core::plugin_credentials::load(&p.manifest.id, item).is_some() {
                rt.configured.insert((p.manifest.id.clone(), item.id.clone()));
            } else if item.required && !missing.contains(&item.id) {
                missing.push(item.id.clone());
            }
        }
        if changed && plugins::save_values(&p.manifest.id, &values).is_err() {
            missing.push("secure-storage".into());
        }
        if !missing.is_empty() {
            rt.missing_credentials.insert(p.manifest.id.clone(), missing);
        }
    }
    let want: Vec<Installed> = installed.iter().filter(|p| may_run(p, &dis) && !rt.missing_credentials.contains_key(&p.manifest.id)).cloned().collect();
    rt.running.retain(|id, (_, version, at)| want.iter().any(|p| &p.manifest.id == id && &p.manifest.version == version && p.origin.installed_at == *at));
    rt.values.retain(|id, _| installed.iter().any(|p| &p.manifest.id == id));
    for p in &installed {
        let mut values = plugins::load_values(&p.manifest.id);
        values.retain(|id, _| p.manifest.item(id).is_some_and(|i| !i.secret));
        rt.values.insert(p.manifest.id.clone(), values);
    }
    let mut started = Vec::new();
    for p in want {
        let id = p.manifest.id.clone();
        if rt.running.contains_key(&id) {
            continue;
        }
        if rt.running.len() >= 16 {
            rt.stopped.insert(id, "at most 16 code plugins may run at once".into());
            continue;
        }
        match plugin_host::start(&p, st.tx.clone(), &server_url) {
            Ok(r) => {
                rt.stopped.remove(&id);
                rt.patches.remove(&id);
                rt.running.insert(id.clone(), (r, p.manifest.version.clone(), p.origin.installed_at));
                started.push(id);
            }
            Err(e) => {
                tracing::warn!(plugin = %id, error = %format!("{e:#}"), "plugin did not start");
                rt.stopped.insert(id, format!("{e:#}"));
            }
        }
    }
    // First event: the plugin's settings and (with `playback`) what plays now.
    for id in started {
        let values = rt.values.get(&id).cloned().unwrap_or_default();
        let sees = rt.installed.iter().any(|p| p.manifest.id == id && p.manifest.grants("playback"));
        send(&rt, &id, json!({
            "type": "ready", "settings": values,
            "song": song.clone().filter(|_| sees), "playing": sees && playing, "lang": lang,
        }));
    }
}

fn send(rt: &Rt, id: &str, event: Value) {
    if let Some((r, _, _)) = rt.running.get(id) {
        r.event(event);
    }
}

/// The song as plugins see it.
fn track_json(core: &Core, t: &glyph_protocol::Track) -> Value {
    let liked = core.st().library.is_liked(t);
    json!({
        "ref": glyph_core::dedupe::track_key(t),
        "title": t.title, "artist": t.artist, "album": t.album,
        "duration_s": t.duration_ms.map(|ms| ms / 1000),
        "explicit": t.explicit, "liked": liked,
    })
}

// ---- JSON for the UI ----

fn theme_json(m: &plugins::Manifest) -> Value {
    match m.theme_colors() {
        Some(c) => json!({ "accent": c.accent, "glow": c.glow, "base": c.base }),
        None => Value::Null,
    }
}

fn permission_list(m: &plugins::Manifest) -> Vec<String> {
    let mut out = m.permissions.clone();
    out.extend(m.network.iter().map(|h| format!("network:{h}")));
    out
}

/// A catalog entry as the UI's PluginListing.
fn listing_json(core: &Core, l: &Listing, installable: &str) -> Value {
    let m = &l.manifest;
    let lang = core.st().config.resolved_language();
    let sections: Vec<String> = m
        .settings
        .iter()
        .map(|s| {
            let page = if s.page == "own" { "own page".to_string() } else { s.page.clone() };
            let title = s.title.get(lang);
            if title.is_empty() { page } else { format!("{title} ({page})") }
        })
        .collect();
    let menus: Vec<String> = m.menus.iter().map(|e| e.label.get(lang).to_string()).collect();
    let setup: Vec<String> = m
        .settings
        .iter()
        .flat_map(|s| &s.items)
        .filter(|i| i.required)
        .map(|i| match &i.credential {
            Some(c) => format!("{} (https://{}{}, {}: {})", i.label.get(lang), c.host, c.path, c.location, c.name),
            None => i.label.get(lang).to_string(),
        })
        .collect();
    json!({
        "id": m.id, "name": m.name, "author": m.author, "description": m.description, "version": m.version,
        "license": m.license, "min_app": m.min_app, "kind": m.kind,
        "repo": l.repo, "path": l.path, "rev": l.rev, "checked": l.checked,
        "stars": l.stars, "updated": l.updated,
        "installable": installable, "code": !m.data_only(),
        "permissions": permission_list(m), "theme": theme_json(m),
        "sections": sections, "menus": menus,
        "strings": m.strings.values().map(|s| s.len()).max().unwrap_or(0),
        "hide": m.hide, "setup": setup, "look": look_json(m),
    })
}

/// What a plugin's look changes (for its card), or null.
fn look_json(m: &plugins::Manifest) -> Value {
    match &m.look {
        Some(l) => json!({
            "font": font_kind(&l.font), "mono": font_kind(&l.mono), "app_icon": l.app_icon.is_some(), "radius": l.radius, "backgrounds": l.backgrounds.len(), "css": l.css.is_some(), "assets": l.assets.len(), "icons": l.icons.len(),
        }),
        None => Value::Null,
    }
}

/// A look's font for the UI: Glyph's font name, or "" for a font file (sent
/// as `font_data` / `mono_data`).
fn font_name(font: &str) -> &str {
    if plugins::is_font_file(font) {
        ""
    } else {
        font
    }
}

/// A look's font for its card: Glyph's font name, `"file"` for its own, or "".
fn font_kind(font: &str) -> &str {
    if plugins::is_font_file(font) {
        "file"
    } else {
        font
    }
}

/// The app icon of the looks in use (.png): a theme's while its theme is
/// picked, an extension's while it is on; the first plugin wins, like the
/// UI's font and corners.
pub(crate) fn active_app_icon(st: &PluginState, dis: &[String], theme: &str) -> Option<Vec<u8>> {
    let rt = st.lock();
    rt.installed
        .iter()
        .filter(|p| !dis.contains(&p.manifest.id))
        .filter(|p| p.manifest.look.as_ref().is_some_and(|l| l.app_icon.is_some()))
        .filter(|p| p.manifest.kind != "theme" || theme == format!("plugin:{}", p.manifest.id))
        .find_map(|p| p.look_app_icon())
}

/// Background pictures of the enabled plugins' looks: (id for
/// `visuals.bg_image`, file, plugin name).
pub fn look_backgrounds(core: &Core) -> Vec<(String, std::path::PathBuf, String)> {
    let Some(st) = core.plugins() else { return Vec::new() };
    let dis = core.st().config.plugins.disabled.clone();
    let rt = st.lock();
    rt.installed
        .iter()
        .filter(|p| !dis.contains(&p.manifest.id))
        .flat_map(|p| p.look_backgrounds().into_iter().map(move |(i, f)| (format!("plugin:{}/{i}", p.manifest.id), f, p.manifest.name.clone())))
        .collect()
}

fn installable_of(l: &Listing) -> &'static str {
    match l.installable() {
        Ok(()) => "ok",
        Err(Invalid::NeedsNewerApp) => "needs_newer_app",
        Err(Invalid::Bad(_)) => "broken",
    }
}

fn catalog_json(core: &Core, c: &Catalog) -> Value {
    let f = |v: &[Listing]| v.iter().map(|l| listing_json(core, l, installable_of(l))).collect::<Vec<_>>();
    json!({ "checked": f(&c.checked), "community": f(&c.community), "partial": c.partial })
}

fn installed_json(core: &Core, rt: &Rt, dis: &[String]) -> Vec<Value> {
    rt.installed
        .iter()
        .map(|p| {
            let m = &p.manifest;
            let cat = rt.catalog.as_ref();
            let from_cat = cat.and_then(|c| c.find(&m.id));
            let listing = Listing {
                repo: p.origin.repo.clone(),
                path: p.origin.path.clone(),
                rev: p.origin.rev.clone(),
                checked: p.origin.checked,
                stars: from_cat.and_then(|l| l.stars),
                updated: from_cat.and_then(|l| l.updated.clone()),
                manifest: m.clone(),
            };
            let installable = match m.validate(plugins::app_version()) {
                Ok(()) => "ok",
                Err(Invalid::NeedsNewerApp) => "needs_newer_app",
                Err(Invalid::Bad(_)) => "broken",
            };
            let mut v = listing_json(core, &listing, installable);
            let has_page = m.main.is_some() && !dis.contains(&m.id) && (p.origin.checked || p.origin.is_local()) && m.sections_on("own").next().is_some();
            let o = v.as_object_mut().expect("object");
            o.insert("local".into(), json!(p.origin.is_local()));
            o.insert("enabled".into(), json!(!dis.contains(&m.id)));
            o.insert("has_page".into(), json!(has_page));
            o.insert("missing_credentials".into(), json!(rt.missing_credentials.contains_key(&m.id)));
            o.insert("stopped".into(), json!(rt.stopped.get(&m.id)));
            o.insert("log".into(), json!(rt.running.get(&m.id).map(|(r, _, _)| r.log_lines()).unwrap_or_default()));
            let update = cat.and_then(|c| p.update_in(c)).map(|l| listing_json(core, l, installable_of(l)));
            o.insert("update".into(), update.unwrap_or(Value::Null));
            v
        })
        .collect()
}

/// One settings row of a plugin; `None` while the plugin hides it.
fn item_json(rt: &Rt, pid: &str, item: &SettingItem, lang: &str) -> Option<Value> {
    let patch = rt.patches.get(pid).and_then(|m| m.get(&item.id)).cloned().unwrap_or_default();
    if patch.get("hidden").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    let text = |k: &str, base: &plugins::Text| -> String { patch.get(k).and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| base.get(lang).to_string()) };
    let label = text("label", &item.label);
    let caption = if item.caption.is_empty() { label.clone() } else { item.caption.get(lang).to_string() };
    let desc = match &item.credential {
        Some(c) => format!("{} · https://{}{} · {}: {}", text("desc", &item.desc), c.host, c.path, c.location, c.name),
        None => text("desc", &item.desc),
    };
    let options: Vec<Value> = match patch.get("options").and_then(Value::as_array) {
        Some(o) => o.clone(),
        None => item.options.iter().map(|o| json!({ "value": o.value, "label": o.label.get(lang) })).collect(),
    };
    let value = if item.secret {
        json!("")
    } else if item.kind == "progress" {
        json!(patch.get("value").and_then(Value::as_f64).unwrap_or(0.0).clamp(0.0, 1.0))
    } else {
        plugins::item_value(item, rt.values.get(pid).unwrap_or(&Map::new()))
    };
    Some(json!({
        "id": item.id, "kind": item.kind, "label": label, "desc": desc, "caption": caption,
        "url": item.url, "unit": item.unit, "min": item.min, "max": item.max, "step": item.step,
        "options": options, "value": value,
        "status": patch.get("status").and_then(Value::as_str).filter(|s| !s.is_empty()),
        "disabled": patch.get("disabled").and_then(Value::as_bool).unwrap_or(false),
        "secret": item.secret,
        "configured": rt.configured.contains(&(pid.to_string(), item.id.clone())),
    }))
}

fn ui_json(rt: &Rt, dis: &[String], lang: &str) -> Value {
    let enabled: Vec<&Installed> = rt.installed.iter().filter(|p| !dis.contains(&p.manifest.id)).collect();
    let manifests: Vec<&plugins::Manifest> = enabled.iter().map(|p| &p.manifest).collect();
    // Texts of the enabled plugins, by language.
    let mut strings: HashMap<String, HashMap<String, String>> = HashMap::new();
    for m in &manifests {
        for (l, map) in &m.strings {
            let t = strings.entry(l.clone()).or_default();
            for (k, v) in map {
                t.insert(k.clone(), v.clone());
            }
        }
    }
    let pages: Vec<Value> = enabled
        .iter()
        .filter(|p| p.manifest.main.is_some() && (p.origin.checked || p.origin.is_local()) && p.manifest.sections_on("own").next().is_some())
        .map(|p| json!({ "id": p.manifest.id, "name": p.manifest.name }))
        .collect();
    // Rows of plugins whose code runs.
    let mut sections = Vec::new();
    for p in rt.installed.iter().filter(|p| rt.running.contains_key(&p.manifest.id)) {
        for s in &p.manifest.settings {
            let items: Vec<Value> = s.items.iter().filter_map(|i| item_json(rt, &p.manifest.id, i, lang)).collect();
            sections.push(json!({
                "plugin": p.manifest.id, "plugin_name": p.manifest.name, "page": s.page,
                "title": s.title.get(lang), "after": s.after, "items": items,
            }));
        }
    }
    // Looks of the enabled plugins (font, corners, icons, style sheet for the whole interface).
    let looks: Vec<Value> = enabled
        .iter()
        .filter_map(|p| {
            let l = p.manifest.look.as_ref()?;
            // A theme's look goes with its colours (the UI shows it while the theme is picked).
            Some(json!({
                "plugin": p.manifest.id, "theme": p.manifest.kind == "theme", "font": font_name(&l.font), "radius": l.radius,
                "font_data": p.look_font_data(&l.font), "mono": font_name(&l.mono), "mono_data": p.look_font_data(&l.mono),
                "css": p.look_css().unwrap_or_default(), "icons": p.look_icons(),
            }))
        })
        .collect();
    json!({ "hidden": plugins::hidden_rows(&manifests), "sections": sections, "pages": pages, "strings": strings, "looks": looks })
}

// ---- commands ----

fn need_plugins(core: &Core) -> Res<Arc<PluginState>> {
    core.plugins().ok_or_else(|| "plugins not ready".to_string())
}

pub fn plugins_state(core: C<'_>) -> Value {
    let Some(st) = core.plugins() else {
        return json!({ "installed": [], "ui": {} });
    };
    let (dis, lang) = {
        let s = core.st();
        (s.config.plugins.disabled.clone(), s.config.resolved_language())
    };
    let rt = st.lock();
    json!({ "installed": installed_json(&core, &rt, &dis), "ui": ui_json(&rt, &dis, lang) })
}

pub async fn plugins_catalog(core: C<'_>, force: Option<bool>) -> Res<Value> {
    let st = need_plugins(core)?;
    let b = core.backend_or_err().map_err(|_| crate::i18n::tr(&core, "plugins.err.offline"))?;
    let c = crate::guard::guard_async(b.plugin_catalog(force.unwrap_or(false))).await?;
    let out = catalog_json(&core, &c);
    st.lock().catalog = Some(c);
    Ok(out)
}

pub async fn plugin_install(core: C<'_>, id: String, repo: String, path: Option<String>, rev: Option<String>) -> Res {
    let st = need_plugins(core)?;
    let core = core.clone();
    let b = core.backend_or_err().map_err(|_| crate::i18n::tr(&core, "plugins.err.offline"))?;
    // Only what the catalog lists can be installed.
    let known = st.lock().catalog.clone();
    let find = |c: &Catalog| c.all().find(|l| l.manifest.id == id && l.repo.eq_ignore_ascii_case(&repo) && path.as_ref().is_none_or(|p| *p == l.path) && rev.as_ref().is_none_or(|r| *r == l.rev)).cloned();
    let listing = match known.as_ref().and_then(find) {
        Some(l) => l,
        None => {
            let c = crate::guard::guard_async(b.plugin_catalog(true)).await?;
            let l = find(&c);
            st.lock().catalog = Some(c);
            l.ok_or_else(|| crate::i18n::tr(&core, "plugins.err.not_found"))?
        }
    };
    // Code and the look's files come from GitHub first (the same commit as the manifest).
    let code = crate::guard::guard_async(b.plugin_code(listing.clone())).await?;
    let look = crate::guard::guard_async(b.plugin_look(listing.clone())).await?;
    let l2 = listing.clone();
    tokio::task::spawn_blocking(move || plugins::install_with_look(&l2, code.as_deref(), &look).map(|_| ()).map_err(|e| format!("{e:#}")))
        .await
        .map_err(|e| e.to_string())??;
    load(&st);
    sync_code(&core, &st);
    crate::theme_icon::sync_host_icon(&core);
    Ok(())
}

pub async fn plugin_uninstall(core: C<'_>, id: String) -> Res {
    let st = need_plugins(core)?;
    let core = core.clone();
    // Stopped before its folder goes.
    st.lock().running.remove(&id);
    let gone = id.clone();
    tokio::task::spawn_blocking(move || plugins::uninstall(&gone).map_err(|e| format!("{e:#}"))).await.map_err(|e| e.to_string())??;
    load(&st);
    sync_code(&core, &st);
    crate::theme_icon::sync_host_icon(&core);
    Ok(())
}

pub fn plugin_enable(core: C<'_>, id: String, on: bool) {
    let Some(st) = core.plugins() else { return };
    {
        let mut s = core.st();
        s.config.plugins.disabled.retain(|d| *d != id);
        if !on {
            s.config.plugins.disabled.push(id);
        }
    }
    core.config_changed();
    load(&st);
    sync_code(&core, &st);
    crate::theme_icon::sync_host_icon(core);
}

/// "Reload" for plugin makers: reads the plugins folder again.
pub fn plugins_reload(core: C<'_>) {
    let Some(st) = core.plugins() else { return };
    {
        let mut rt = st.lock();
        let local: Vec<String> = rt.installed.iter().filter(|p| p.origin.is_local()).map(|p| p.manifest.id.clone()).collect();
        rt.running.retain(|id, _| !local.contains(id));
    }
    load(&st);
    // Reload also re-reads what is installed (an edited manifest).
    st.lock().installed = plugins::installed();
    sync_code(&core, &st);
    crate::theme_icon::sync_host_icon(core);
}

/// Creates the plugins folder and shows it.
pub fn plugins_folder() -> Res {
    let dir = plugins::plugins_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    crate::host::get().open_path(&dir)
}

/// The user changed a plugin's settings row.
pub fn plugin_set_value(core: C<'_>, plugin: String, item: String, value: Value) {
    let Some(st) = core.plugins() else { return };
    let p = st.lock().installed.iter().find(|p| p.manifest.id == plugin).cloned();
    let Some(p) = p else { return };
    let Some(setting) = p.manifest.item(&item) else { return };
    if setting.secret {
        match glyph_core::plugin_credentials::store(&plugin, setting, value.as_str().unwrap_or("")) {
            Ok(()) => {
                sync_code(&core, &st);
                send(&st.lock(), &plugin, json!({ "type": "credential", "id": item, "configured": !value.as_str().unwrap_or("").is_empty() }));
            }
            Err(_) => core.toast_key("plugins.setup.save_failed", "info"),
        }
        return;
    }
    let Some(v) = plugins::coerce_value(setting, &value) else { return };
    store_value(&st, &plugin, &item, v.clone());
    send(&st.lock(), &plugin, json!({ "type": "setting", "id": item, "value": v }));
}

fn store_value(st: &PluginState, id: &str, item: &str, value: Value) {
    let mut rt = st.lock();
    let values = rt.values.entry(id.to_string()).or_default();
    values.insert(item.to_string(), value);
    if let Err(e) = plugins::save_values(id, values) {
        tracing::warn!(plugin = %id, error = %format!("{e:#}"), "saving plugin settings");
    }
}

/// A plugin's button was pressed.
pub fn plugin_action(core: C<'_>, plugin: String, item: String) {
    let Some(st) = core.plugins() else { return };
    send(&st.lock(), &plugin, json!({ "type": "action", "id": item }));
}

/// A plugin page / search / artist / install prompt for the UI to open
/// (event `plugin_ui`, see `settings/plugin-events.svelte.ts`).
fn ui_event(core: &Core, event: Value) {
    core.emit("plugin_ui", event);
}

/// A `glyph://plugin/<id>` link: Settings > Plugins on that plugin, with the
/// install prompt when it is not installed (GPUI `open_plugin`).
pub fn open_plugin(core: &Arc<Core>, id: &str) {
    ui_event(core, json!({ "kind": "install", "id": id }));
}

/// The song key the UI uses (`trackKey` in types.ts).
fn ui_key(t: &glyph_protocol::Track) -> String {
    format!("{}|{}", t.title.trim().to_lowercase(), t.artist.trim().to_lowercase())
}

/// Lyrics of running plugins for the song with this key.
fn plugin_lyrics_of(st: &PluginState, key: &str) -> Vec<glyph_core::lyrics::Lyrics> {
    let rt = st.lock();
    let mut v: Vec<_> = rt.lyrics.iter().filter(|(id, (k, _))| rt.running.contains_key(*id) && k == key).map(|(id, (_, l))| (id.clone(), l.clone())).collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v.into_iter().map(|x| x.1).collect()
}

/// The saved timing offset of the playing song when `key` is its key.
fn offset_for(core: &Core, key: &str) -> i64 {
    let st = core.st();
    st.current.as_ref().filter(|t| ui_key(t) == key).map_or(0, |t| st.library.lyrics_offset(t))
}

/// Lyrics plugins gave for the song with this key (the UI shows them when
/// nothing else has lyrics, and as choices).
pub fn plugin_lyrics(core: C<'_>, key: String) -> Vec<Value> {
    let Some(st) = core.plugins() else { return Vec::new() };
    let off = offset_for(&core, &key);
    plugin_lyrics_of(&st, &key).iter().map(|l| crate::dto::lyrics(l, off)).collect()
}

/// For the MCP tool `get_lyrics`: the first plugin lyrics of the playing song.
pub fn first_lyrics(core: &Core) -> Option<glyph_core::lyrics::Lyrics> {
    let key = core.st().current.as_ref().map(ui_key)?;
    let st = core.plugins()?;
    plugin_lyrics_of(&st, &key).into_iter().next()
}

fn plugin_lyrics_parse(id: &str, name: &str, args: &Value) -> Result<glyph_core::lyrics::Lyrics, String> {
    let plain = args["plain"].as_str().map(str::to_string);
    let lrc = args["lrc"].as_str().unwrap_or("");
    if plain.as_ref().map_or(0, |s| s.len()) + lrc.len() > 128 * 1024 {
        return Err("lyrics exceed 128 KiB".into());
    }
    // LRC permits several timestamps per line; bound expansion before parsing.
    if lrc.matches('[').count() > 2000 || lrc.lines().any(|l| l.len() > 4000) {
        return Err("too many timestamps or an oversized lyrics line".into());
    }
    let synced = glyph_core::lyrics::Lyrics::parse_lrc(lrc);
    if synced.len() > 2000 || synced.iter().any(|l| l.time_ms > 24 * 3600 * 1000) {
        return Err("invalid lyrics timing or too many lines".into());
    }
    if plain.as_ref().is_none_or(|s| s.trim().is_empty()) && synced.is_empty() {
        return Err("lyrics need plain text or timed LRC".into());
    }
    Ok(glyph_core::lyrics::Lyrics { provider: format!("Plugin: {name}"), match_score: 1.0, synced, plain, instrumental: false, source_id: format!("plugin:{id}") })
}

/// While a jam runs, plugins may not steer playback. The rule belongs to
/// the jam (`crate::jam::allows_control` once it is there); a guest never controls.
fn control_allowed(_core: &Arc<Core>) -> bool {
    crate::jam::allows_control()
}

// ---- plugin code at work ----

fn ok(v: Value) -> Value {
    json!({ "ok": v })
}

fn err(msg: impl Into<String>) -> Value {
    json!({ "error": msg.into() })
}

/// An MCP tool result as a plugin answer.
fn from_mcp(r: Value) -> Value {
    let text = r["content"][0]["text"].as_str().unwrap_or("").to_string();
    if r["isError"].as_bool().unwrap_or(false) {
        return err(text);
    }
    match r.get("structuredContent") {
        Some(v) => ok(v.clone()),
        None => ok(Value::String(text)),
    }
}

/// Shallow merge: `patch`'s fields replace `base`'s (both objects), else `patch` wins.
fn merge(base: &Value, patch: &Value) -> Value {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            let mut out = b.clone();
            for (k, v) in p {
                out.insert(k.clone(), v.clone());
            }
            Value::Object(out)
        }
        _ => patch.clone(),
    }
}

/// Answers calls that arrived and tells plugins what changed (10 times a second).
fn poll(core: &Arc<Core>, st: &Arc<PluginState>) {
    let url = core.st().config.effective_server_url().to_string();
    if st.lock().server_url != url {
        sync_code(core, st);
    }
    let calls: Vec<HostCall> = st.rx.try_iter().take(32).collect();
    for c in calls {
        call(core, st, c);
    }
    if st.lock().running.is_empty() {
        return;
    }
    let cur = core.st().current.clone();
    let key = cur.as_ref().map(glyph_core::dedupe::track_key);
    let playing = core.player_state().is_some_and(|s| s.status == glyph_audio::Status::Playing);
    let (track_changed, play_changed) = {
        let mut rt = st.lock();
        let t = rt.last_track != key;
        rt.last_track = key;
        let p = rt.last_playing != playing;
        rt.last_playing = playing;
        (t, p)
    };
    if track_changed {
        let song = cur.as_ref().map(|t| track_json(core, t));
        broadcast(st, json!({ "type": "track", "song": song }));
    }
    if play_changed {
        broadcast(st, json!({ "type": "playback", "playing": playing }));
    }
}

/// Sends a playback event to the plugins allowed to see it.
fn broadcast(st: &PluginState, event: Value) {
    let rt = st.lock();
    for (id, (r, _, _)) in &rt.running {
        if rt.installed.iter().any(|p| &p.manifest.id == id && p.manifest.grants("playback")) {
            r.event(event.clone());
        }
    }
}

fn call(core: &Arc<Core>, st: &Arc<PluginState>, c: HostCall) {
    let HostCall { plugin: id, generation, func, args, reply } = c;
    let dis = disabled(core);
    let p = {
        let rt = st.lock();
        if !rt.running.get(&id).is_some_and(|(r, _, _)| r.generation == generation) {
            let _ = reply.send(err("the plugin invocation is no longer active"));
            return;
        }
        rt.installed.iter().find(|p| p.manifest.id == id).cloned()
    };
    let Some(p) = p else {
        let _ = reply.send(err("the plugin is not installed"));
        return;
    };
    if func == plugin_host::STOPPED {
        {
            let mut rt = st.lock();
            rt.running.remove(&id);
            rt.stopped.insert(id.clone(), args["error"].as_str().unwrap_or("").to_string());
        }
        core.emit("toast", json!({ "key": "plugins.stopped_toast", "text": "plugins.stopped_toast", "vars": { "name": p.manifest.name }, "kind": "info" }));
        return;
    }
    if !may_run(&p, &dis) || !glyph_core::plugin_policy::known_function(&func) {
        let _ = reply.send(err("plugin function is disabled or unavailable"));
        return;
    }
    if let Some(perm) = plugin_host::permission_for(&func) {
        if !p.manifest.grants(perm) {
            let _ = reply.send(err(format!("{func} needs the \"{perm}\" permission")));
            return;
        }
    }
    if (func.starts_with("player.") && func != "player.state" || func.starts_with("queue.") && func != "queue.get" || func == "library.play_playlist") && !control_allowed(core) {
        let _ = reply.send(err("plugin playback control is unavailable during a Jam"));
        return;
    }
    if !st.lock().budget.admit(&func) {
        let _ = reply.send(err("combined plugin call limit reached; try again later"));
        return;
    }
    if let Some((_, tool)) = MCP_FUNCS.iter().find(|(f, _)| *f == func) {
        // A tool may search (seconds): its own thread, the plugin loop goes on.
        let (core, tool, func) = (core.clone(), tool.to_string(), func.clone());
        crate::rt::spawn_blocking(move || {
            let answer = match MCP.get().and_then(|h| h(&core, &tool, &args)) {
                Some(v) => from_mcp(v),
                None => basic_player(&core, &func),
            };
            let _ = reply.send(answer);
        });
        return;
    }
    let _ = reply.send(other_call(core, st, &p, &func, &args));
}

/// Player functions without an MCP handler: the simple ones.
fn basic_player(core: &Arc<Core>, func: &str) -> Value {
    let status = core.player_state().map(|s| s.status);
    match func {
        "player.state" => {
            let cur = core.st().current.clone().map(|t| track_json(core, &t));
            let ps = core.player_state();
            let (shuffle, repeat) = {
                let s = core.st();
                (s.config.player.shuffle, s.config.player.repeat.clone())
            };
            ok(json!({
                "song": cur,
                "playing": status == Some(glyph_audio::Status::Playing),
                "position_s": ps.as_ref().map(|s| s.position.as_secs()),
                "duration_s": ps.as_ref().and_then(|s| s.duration).map(|d| d.as_secs()),
                "volume": ps.as_ref().map(|s| (s.volume * 100.0).round()),
                "shuffle": shuffle, "repeat": repeat,
            }))
        }
        "player.play" => {
            if status != Some(glyph_audio::Status::Playing) {
                core.toggle_play();
            }
            ok(Value::Null)
        }
        "player.pause" => {
            if status == Some(glyph_audio::Status::Playing) {
                core.toggle_play();
            }
            ok(Value::Null)
        }
        "player.toggle" => {
            core.toggle_play();
            ok(Value::Null)
        }
        "player.next" => {
            core.next(false);
            ok(Value::Null)
        }
        "player.previous" => {
            core.previous();
            ok(Value::Null)
        }
        _ => err(format!("{func} is not available yet")),
    }
}

fn other_call(core: &Arc<Core>, st: &Arc<PluginState>, p: &Installed, func: &str, args: &Value) -> Value {
    let id = &p.manifest.id;
    match func {
        "app.info" => ok(json!({
            "version": env!("CARGO_PKG_VERSION"),
            "platform": std::env::consts::OS,
            "lang": core.st().config.resolved_language(),
            "theme": core.st().config.visuals.theme,
            "plugin": { "id": id, "version": p.manifest.version },
        })),
        "app.text" => match args["key"].as_str().filter(|k| !k.is_empty() && k.len() <= 100) {
            Some(k) => ok(Value::String(crate::i18n::tr(core, k))),
            None => err("app.text needs a key"),
        },
        "ui.toast" => {
            let text: String = args["text"].as_str().unwrap_or("").trim().chars().take(200).collect();
            if text.is_empty() {
                err("ui.toast needs a text")
            } else {
                // Named, so nobody mistakes a plugin's message for Glyph's.
                let kind = if args["error"].as_bool().unwrap_or(false) { "info" } else { "ok" };
                core.emit("toast", json!({ "text": format!("{}: {text}", p.manifest.name), "kind": kind }));
                ok(Value::Null)
            }
        }
        "ui.get" => match args["id"].as_str().and_then(|i| p.manifest.item(i)) {
            Some(item) if item.secret => {
                ok(json!({ "configured": st.lock().configured.contains(&(id.clone(), item.id.clone())) }))
            }
            Some(item) => ok(plugins::item_value(item, st.lock().values.get(id).unwrap_or(&Map::new()))),
            None => err("no settings item with this id"),
        },
        "ui.values" => {
            let rt = st.lock();
            let values = rt.values.get(id).cloned().unwrap_or_default();
            let all: Map<String, Value> = p
                .manifest
                .settings
                .iter()
                .flat_map(|s| s.items.iter())
                .filter(|i| !i.secret && !matches!(i.kind.as_str(), "button" | "note" | "link" | "progress"))
                .map(|i| (i.id.clone(), plugins::item_value(i, &values)))
                .collect();
            ok(Value::Object(all))
        }
        "ui.set" => match args["id"].as_str().and_then(|i| p.manifest.item(i)).filter(|i| !i.secret).and_then(|item| plugins::coerce_value(item, &args["value"]).map(|v| (item.id.clone(), v))) {
            Some((item, v)) => {
                store_value(st, id, &item, v);
                ok(Value::Null)
            }
            None => err("no settings item with this id, or the value does not fit it"),
        },
        "ui.update" => match args["id"].as_str().filter(|i| p.manifest.item(i).is_some()) {
            Some(item) => {
                if p.manifest.item(item).is_some_and(|i| i.secret) {
                    return err("credential controls are managed by Glyph");
                }
                let mut patch = Map::new();
                if let Some(v) = args["value"].as_f64().filter(|v| v.is_finite()) {
                    patch.insert("value".into(), json!(v.clamp(0.0, 1.0)));
                }
                for k in ["label", "desc", "status"] {
                    if let Some(v) = args.get(k) {
                        patch.insert(k.into(), Value::String(v.as_str().unwrap_or("").chars().take(300).collect()));
                    }
                }
                for k in ["hidden", "disabled"] {
                    if let Some(v) = args[k].as_bool() {
                        patch.insert(k.into(), Value::Bool(v));
                    }
                }
                if let Some(opts) = args["options"].as_array() {
                    let opts: Vec<Value> = opts
                        .iter()
                        .filter_map(|o| {
                            let v = o["value"].as_str()?;
                            Some(json!({ "value": v.chars().take(60).collect::<String>(), "label": o["label"].as_str().unwrap_or(v).chars().take(60).collect::<String>() }))
                        })
                        .take(50)
                        .collect();
                    patch.insert("options".into(), Value::Array(opts));
                }
                let mut rt = st.lock();
                let entry = rt.patches.entry(id.clone()).or_default().entry(item.to_string()).or_default();
                for (k, v) in patch {
                    entry.insert(k, v);
                }
                ok(Value::Null)
            }
            None => err("no settings item with this id"),
        },
        "ui.open" => {
            if !disabled(core).contains(id) && p.manifest.sections_on("own").next().is_some() {
                ui_event(core, json!({ "kind": "page", "id": id }));
                ok(Value::Null)
            } else {
                err("the plugin has no settings page")
            }
        }
        "ui.search" => match args["query"].as_str().map(str::trim).filter(|q| !q.is_empty() && q.len() <= 200) {
            Some(q) => {
                ui_event(core, json!({ "kind": "search", "query": q }));
                ok(Value::Null)
            }
            None => err("ui.search needs a query"),
        },
        "ui.open_artist" => match args["name"].as_str().map(str::trim).filter(|n| !n.is_empty() && n.len() <= 200) {
            Some(n) => {
                ui_event(core, json!({ "kind": "artist", "name": n }));
                ok(Value::Null)
            }
            None => err("ui.open_artist needs a name"),
        },
        "lyrics.publish" => {
            let cur = core.st().current.clone();
            match cur.filter(|t| args["ref"].as_str() == Some(glyph_core::dedupe::track_key(t).as_str())) {
                Some(t) => match plugin_lyrics_parse(id, &p.manifest.name, args) {
                    Ok(l) => {
                        let key = ui_key(&t);
                        st.lock().lyrics.insert(id.clone(), (key.clone(), l.clone()));
                        let off = core.st().library.lyrics_offset(&t);
                        core.emit("plugin_lyrics", json!({ "key": key, "lyrics": crate::dto::lyrics(&l, off) }));
                        ok(Value::Null)
                    }
                    Err(e) => err(e),
                },
                None => err("lyrics belong to a different song"),
            }
        }
        "audio.spectrum" => {
            let bands = core.player.as_ref().map(|p| p.spectrum().to_vec()).unwrap_or_else(|| vec![0.0; glyph_audio::SPECTRUM_BANDS]);
            let playing = core.player_state().is_some_and(|s| s.status == glyph_audio::Status::Playing);
            ok(json!({ "bands": bands, "playing": playing }))
        }
        "audio.equalizer" => {
            let s = core.st();
            ok(json!({ "enabled": s.config.audio.eq_enabled, "gains": s.config.audio.eq_gains }))
        }
        "audio.set_equalizer" => {
            let gains = args["gains"]
                .as_array()
                .filter(|v| v.len() == 10)
                .and_then(|v| v.iter().map(|x| x.as_f64().filter(|x| x.is_finite() && (-12.0..=12.0).contains(x)).map(|x| x as f32)).collect::<Option<Vec<_>>>())
                .and_then(|v| <[f32; 10]>::try_from(v).ok());
            match (gains, args["enabled"].as_bool()) {
                (Some(gains), Some(enabled)) => {
                    {
                        let mut s = core.st();
                        s.config.audio.eq_enabled = enabled;
                        s.config.audio.eq_preset = "custom".into();
                        s.config.audio.eq_gains = gains.to_vec();
                    }
                    if let Some(pl) = &core.player {
                        pl.set_equalizer(enabled, gains);
                    }
                    core.config_changed();
                    ok(Value::Null)
                }
                _ => err("expected enabled and ten finite gains between -12 and 12 dB"),
            }
        }
        "queue.remove" | "queue.move" => {
            if crate::jam::jam_active() {
                return err("plugins cannot edit a Jam queue");
            }
            let n = core.st().queue.upcoming().len() as u64;
            match (args["index"].as_u64(), args["from"].as_u64(), args["to"].as_u64()) {
                (Some(i), _, _) if func == "queue.remove" && i < n => {
                    core.queue_remove(i as usize);
                    ok(Value::Null)
                }
                (_, Some(a), Some(b)) if func == "queue.move" && a < n && b < n => {
                    core.queue_move(a as usize, b as usize);
                    ok(Value::Null)
                }
                _ => err("queue index is out of bounds"),
            }
        }
        "settings.get" => {
            let all = glyph_core::settings_share::sections(&core.st().config);
            match args["section"].as_str() {
                Some(s) => all.get(s).cloned().map(ok).unwrap_or_else(|| err(format!("no settings section {s}"))),
                None => ok(Value::Object(all)),
            }
        }
        "settings.set" => {
            let (Some(section), Some(value)) = (args["section"].as_str(), args.get("value")) else {
                return err("settings.set needs a section and a value");
            };
            if !matches!(section, "general" | "visuals" | "audio" | "playback" | "player" | "lists" | "eq_presets") {
                return err("plugins cannot change server, provider, integration, privacy or update policy");
            }
            let mut cfg = core.st().config.clone();
            let all = glyph_core::settings_share::sections(&cfg);
            let Some(cur) = all.get(section) else { return err(format!("no settings section {section}")) };
            let merged = merge(cur, value);
            let mut one = Map::new();
            one.insert(section.to_string(), merged);
            let r = glyph_core::settings_share::apply_sections(&mut cfg, one);
            if r.changed {
                crate::settings::apply_to_player(core, &cfg);
                let (shuffle, repeat) = (cfg.player.shuffle, cfg.player.repeat.clone());
                let before = {
                    let s = core.st();
                    (s.config.player.shuffle, s.config.player.repeat.clone())
                };
                core.st().config = cfg;
                if before.0 != shuffle {
                    core.set_shuffle(shuffle);
                }
                if before.1 != repeat {
                    core.set_repeat(&repeat);
                }
                core.config_changed();
            }
            ok(json!({ "changed": r.changed }))
        }
        other => err(format!("unknown function {other}")),
    }
}
