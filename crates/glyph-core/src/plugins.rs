//! Plugins people publish on GitHub. Two kinds share one manifest, catalog
//! and install flow:
//! - **theme**: colours only (pure data).
//! - **extension**: anything else. Without code it can rename texts
//!   (`strings`), hide settings rows (`hide`) and bring a theme. With code
//!   (`main`, a WebAssembly module run in a sandbox by
//!   [`crate::plugin_host`]) it can also add settings pages and sections,
//!   song menu entries, and use Glyph's API (playback, library, search,
//!   settings, network) as far as its `permissions` allow.
//!
//! Discovery runs on this device, never through a Glyph server:
//! - **Checked**: `index.json` of the `glyphmusic/plugins` repo. Authors add
//!   their plugin there by pull request; CI checks the manifest and pins the
//!   commit.
//! - **Community**: every GitHub repo with the topic `glyph-plugin`, found
//!   with GitHub's search API. Nobody checked these; only plugins without
//!   code may be installed from here.
//!
//! The catalog is cached in `<data>/plugins/catalog.json` for an hour (the
//! unauthenticated search API allows 10 calls a minute). Installed plugins
//! live in `<data>/plugins/<id>/` (`glyph-plugin.json` + `origin.json`, the
//! code as `code.wasm`, the plugin's settings in `values.json` and its own
//! data in `storage.json`).

use crate::paths;
use anyhow::{anyhow, bail, Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// GitHub topic that marks a repo as a Glyph plugin.
pub const TOPIC: &str = "glyph-plugin";
/// Manifest file at the root of a plugin repo (or of its `path`).
pub const MANIFEST_FILE: &str = "glyph-plugin.json";
/// Checked plugins (built by CI in `glyphmusic/plugins`).
pub const INDEX_URL: &str = "https://raw.githubusercontent.com/glyphmusic/plugins/main/index.json";
/// Where authors read how to publish a plugin.
pub const GUIDE_URL: &str = "https://github.com/glyphmusic/plugins#readme";
/// The catalog on the website (each plugin at `/plugins/<id>`).
pub const WEB_URL: &str = "https://glyph.my/plugins";
/// Highest manifest schema this build understands (1: themes, 2: extensions).
pub const SCHEMA: u32 = 2;
/// Installed plugin code, next to the manifest.
pub const CODE_FILE: &str = "code.wasm";
/// Plugin code bigger than this is refused.
pub const MAX_CODE: usize = 16 * 1024 * 1024;
/// Placeholder id of glyphmusic/plugin-template; never a real plugin.
pub const TEMPLATE_ID: &str = "change-me";

const SEARCH_URL: &str = "https://api.github.com/search/repositories?q=topic:glyph-plugin+fork:true&sort=updated&order=desc&per_page=100";
const CATALOG_TTL_S: u64 = 3600;
/// Community repos whose manifest is fetched per refresh (newest first).
const COMMUNITY_MAX: usize = 60;
/// Manifests are small (texts in a few languages at most); anything bigger
/// is not one.
const MAX_MANIFEST: usize = 256 * 1024;

// ------------------------------------------------------------------ manifest

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default = "schema_one")]
    pub schema: u32,
    /// Lowercase `a-z0-9-`, unique across all plugins.
    pub id: String,
    pub name: String,
    /// Semantic version (`1.2.0`); a higher one in the catalog is an update.
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub license: String,
    /// Oldest Glyph version the plugin works with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_app: Option<String>,
    /// `"theme"` or `"extension"`; other kinds need a newer Glyph.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<ThemeSpec>,
    /// WebAssembly module (path in the repo, next to the manifest); the
    /// plugin's code. Without it the plugin is pure data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main: Option<String>,
    /// What the code may use (see [`PERMISSIONS`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<String>,
    /// Hosts the code may contact (with the `network` permission), like
    /// `api.example.com` or `*.example.com`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub network: Vec<String>,
    /// Texts by language code: the plugin's own (keys starting with
    /// `<id>.`) and replacements for Glyph's (any other key).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub strings: BTreeMap<String, BTreeMap<String, String>>,
    /// Settings rows of Glyph to hide, by their label key.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hide: Vec<String>,
    /// Settings the plugin adds: its own page and sections on Glyph's pages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub settings: Vec<SettingsSection>,
    /// Entries the plugin adds to menus (song menu).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub menus: Vec<MenuEntry>,
}

fn schema_one() -> u32 {
    1
}

/// Permissions a plugin's code can ask for.
pub const PERMISSIONS: &[&str] = &["playback", "library", "search", "settings", "network", "audio", "lyrics_provider"];

/// Glyph's settings pages plugins can add sections to (`page.subtab`), plus
/// `"own"`: the plugin's own page under Settings › Plugins.
pub const PAGES: &[&str] = &[
    "own",
    "playback",
    "playback.content",
    "playback.sources",
    "audio",
    "appearance",
    "integrations",
    "storage",
    "system",
    "about",
];

/// Settings controls a plugin can add.
pub const ITEM_TYPES: &[&str] = &["toggle", "slider", "select", "text", "button", "note", "link", "progress"];

/// Glyph texts plugins may neither replace nor hide: the plugin warnings,
/// the account and assistant access.
const PROTECTED_KEYS: &[&str] = &["plugins.", "account.", "mcp."];

/// A text in one or several languages: `"Hello"` or
/// `{"en": "Hello", "de": "Hallo"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Text {
    Plain(String),
    ByLang(BTreeMap<String, String>),
}

impl Default for Text {
    fn default() -> Self {
        Text::Plain(String::new())
    }
}

impl Text {
    /// In `lang`, else English, else any.
    pub fn get(&self, lang: &str) -> &str {
        match self {
            Text::Plain(s) => s,
            Text::ByLang(m) => m.get(lang).or_else(|| m.get("en")).or_else(|| m.values().next()).map(String::as_str).unwrap_or(""),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Text::Plain(s) => s.trim().is_empty(),
            Text::ByLang(m) => m.values().all(|s| s.trim().is_empty()),
        }
    }

    fn longest(&self) -> usize {
        match self {
            Text::Plain(s) => s.chars().count(),
            Text::ByLang(m) => m.values().map(|s| s.chars().count()).max().unwrap_or(0),
        }
    }

    fn langs_ok(&self) -> bool {
        match self {
            Text::Plain(_) => true,
            Text::ByLang(m) => m.keys().all(|k| valid_lang(k)),
        }
    }
}

/// A group of settings rows from a plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingsSection {
    pub id: String,
    /// One of [`PAGES`].
    #[serde(default = "own_page")]
    pub page: String,
    /// Label key of the Glyph row this section follows (`sp.crossfade`);
    /// empty or not on the page: at the end of the page.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub after: String,
    #[serde(default, skip_serializing_if = "Text::is_empty")]
    pub title: Text,
    #[serde(default)]
    pub items: Vec<SettingItem>,
}

fn own_page() -> String {
    "own".into()
}

/// One settings row from a plugin. Its value is kept by Glyph (and sent to
/// the plugin when it changes).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingItem {
    pub id: String,
    /// One of [`ITEM_TYPES`].
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Text::is_empty")]
    pub label: Text,
    /// Explanation behind the (i).
    #[serde(default, skip_serializing_if = "Text::is_empty")]
    pub desc: Text,
    /// Starting value (toggle: bool, slider: number, select/text: string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    /// Shown after a slider's value (`%`, `s`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SelectOption>,
    #[serde(default, skip_serializing_if = "Text::is_empty")]
    pub placeholder: Text,
    /// Caption on a `button` or `link` (default: the label).
    #[serde(default, skip_serializing_if = "Text::is_empty")]
    pub caption: Text,
    /// A `text` row that hides what is typed (passwords, tokens).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub secret: bool,
    /// A required secret must be configured before the plugin's code starts.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
    /// Exact destination and field where Glyph may inject this credential.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<CredentialSpec>,
    /// `link` rows: the page to open (https).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectOption {
    pub value: String,
    pub label: Text,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialSpec {
    pub host: String,
    pub path: String,
    pub location: String,
    pub name: String,
}

/// A menu entry from a plugin; clicking it sends the plugin a `menu` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MenuEntry {
    pub id: String,
    pub label: Text,
    /// Which menu: `"song"`.
    #[serde(default = "song_menu")]
    pub menu: String,
}

fn song_menu() -> String {
    "song".into()
}

/// Two-letter language code (`en`, `de`).
fn valid_lang(code: &str) -> bool {
    code.len() == 2 && code.bytes().all(|b| b.is_ascii_lowercase())
}

/// Ids of settings sections, items and menu entries: 1-40 of `a-z0-9_-`.
fn valid_key(id: &str) -> bool {
    (1..=40).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// Glyph text keys (`sp.crossfade`).
fn valid_text_key(key: &str) -> bool {
    (1..=100).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

fn protected(key: &str) -> bool {
    PROTECTED_KEYS.iter().any(|p| key.starts_with(p))
}

/// `api.example.com` or `*.example.com`: lowercase DNS name, no port or path.
pub fn valid_host(host: &str) -> bool {
    let name = host.strip_prefix("*.").unwrap_or(host);
    (3..=253).contains(&name.len())
        && name.contains('.')
        && name.split('.').all(|l| !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-') && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
}

/// `host` (as in a URL) is covered by one of `allowed`.
pub fn host_allowed(allowed: &[String], host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    allowed.iter().any(|a| match a.strip_prefix("*.") {
        Some(base) => host == base || host.ends_with(&format!(".{base}")),
        None => host == *a,
    })
}

/// Colours of a theme plugin, as `#rrggbb`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeSpec {
    /// Accent (buttons, progress, highlights).
    pub accent: String,
    /// Background glow while nothing plays, dark -> bright. Missing: the
    /// views' own colours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow: Option<[String; 3]>,
    /// Near-black window base. Missing: Glyph's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
}

/// Parsed theme colours (`0xrrggbb`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeColors {
    pub accent: u32,
    pub glow: Option<[u32; 3]>,
    pub base: u32,
}

pub const DEFAULT_BASE: u32 = 0x030303;
/// Brightest channel a base may have: text is drawn light on it.
const MAX_BASE_CHANNEL: u32 = 0x24;

/// `#rrggbb` (or `rrggbb`) -> `0xrrggbb`.
pub fn parse_hex(s: &str) -> Option<u32> {
    let h = s.trim().strip_prefix('#').unwrap_or(s.trim());
    (h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit())).then(|| u32::from_str_radix(h, 16).ok()).flatten()
}

impl ThemeSpec {
    pub fn colors(&self) -> Result<ThemeColors> {
        let accent = parse_hex(&self.accent).ok_or_else(|| anyhow!("theme.accent is not a #rrggbb colour"))?;
        let glow = match &self.glow {
            Some(g) => {
                let mut out = [0u32; 3];
                for (o, s) in out.iter_mut().zip(g) {
                    *o = parse_hex(s).ok_or_else(|| anyhow!("theme.glow has a colour that is not #rrggbb"))?;
                }
                Some(out)
            }
            None => None,
        };
        let base = match &self.base {
            Some(b) => parse_hex(b).ok_or_else(|| anyhow!("theme.base is not a #rrggbb colour"))?,
            None => DEFAULT_BASE,
        };
        if [base >> 16, (base >> 8) & 0xff, base & 0xff].iter().any(|c| *c > MAX_BASE_CHANNEL) {
            bail!("theme.base must stay near black (each channel at most #{MAX_BASE_CHANNEL:02x})");
        }
        Ok(ThemeColors { accent, glow, base })
    }
}

/// Why a manifest cannot be installed.
#[derive(Debug, Clone, PartialEq)]
pub enum Invalid {
    /// Written for a newer Glyph (schema, kind or `min_app`).
    NeedsNewerApp,
    /// Broken or against the rules; the text says what.
    Bad(String),
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Invalid::NeedsNewerApp => f.write_str("needs a newer Glyph"),
            Invalid::Bad(s) => f.write_str(s),
        }
    }
}

/// Plugin ids: 2-40 of `a-z0-9-`, no leading/trailing/double dash.
pub fn valid_id(id: &str) -> bool {
    (2..=40).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-')
        && !id.contains("--")
}

/// `owner/name` as GitHub allows it.
pub fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let ok = |s: &str| !s.is_empty() && s.len() <= 100 && s != "." && s != ".." && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    matches!((parts.next(), parts.next(), parts.next()), (Some(o), Some(n), None) if ok(o) && ok(n))
}

/// Sub-folder of a repo (`themes/sunset`); empty = the root.
fn valid_path(path: &str) -> bool {
    path.is_empty()
        || (path.len() <= 200
            && path.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_./".contains(&b))
            && path.split('/').all(|s| !s.is_empty() && s != "." && s != ".."))
}

impl Manifest {
    /// Checks everything Glyph relies on; `app` is the running version.
    pub fn validate(&self, app: &str) -> Result<(), Invalid> {
        let bad = |s: &str| Err(Invalid::Bad(s.to_string()));
        if self.schema > SCHEMA {
            return Err(Invalid::NeedsNewerApp);
        }
        if self.schema == 0 {
            return bad("schema must be 1 or 2");
        }
        if !valid_id(&self.id) {
            return bad("id must be 2-40 characters of a-z, 0-9 and single dashes");
        }
        if self.id == TEMPLATE_ID {
            return bad("give the plugin its own id (\"change-me\" is the template's)");
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 40 {
            return bad("name must be 1-40 characters");
        }
        if self.author.chars().count() > 40 || self.description.chars().count() > 200 || self.license.chars().count() > 40 {
            return bad("author/license may have 40 characters, description 200");
        }
        if semver::Version::parse(&self.version).is_err() {
            return bad("version must be a semantic version like 1.0.0");
        }
        if let Some(min) = &self.min_app {
            let min = semver::Version::parse(min).map_err(|_| Invalid::Bad("min_app must be a semantic version".into()))?;
            if semver::Version::parse(app).is_ok_and(|v| v < min) {
                return Err(Invalid::NeedsNewerApp);
            }
        }
        if let Some(t) = &self.theme {
            t.colors().map_err(|e| Invalid::Bad(e.to_string()))?;
        }
        match self.kind.as_str() {
            "theme" => {
                if self.theme.is_none() {
                    return bad("a theme plugin needs a \"theme\" object");
                }
                if self.main.is_some() || !self.strings.is_empty() || !self.hide.is_empty() || !self.settings.is_empty() || !self.menus.is_empty() {
                    return bad("a theme has colours only; use \"kind\": \"extension\" for more");
                }
                Ok(())
            }
            "extension" => {
                if self.schema < 2 {
                    return bad("extensions need \"schema\": 2");
                }
                self.validate_extension()
            }
            _ => Err(Invalid::NeedsNewerApp),
        }
    }

    fn validate_extension(&self) -> Result<(), Invalid> {
        let bad = |s: String| Err(Invalid::Bad(s));
        if self.theme.is_none() && self.main.is_none() && self.strings.is_empty() && self.hide.is_empty() {
            return bad("an extension needs code (\"main\"), \"strings\", \"hide\" or a \"theme\"".into());
        }
        if let Some(main) = &self.main {
            if !valid_path(main) || main.is_empty() || !main.ends_with(".wasm") {
                return bad("main must be the path of a .wasm file in the repo".into());
            }
        } else if !self.settings.is_empty() || !self.menus.is_empty() || !self.permissions.is_empty() {
            return bad("settings, menus and permissions need code (\"main\")".into());
        }
        // A permission this build does not know comes from a newer plugin API.
        if self.permissions.iter().any(|p| !PERMISSIONS.contains(&p.as_str())) {
            return Err(Invalid::NeedsNewerApp);
        }
        let network = self.permissions.iter().any(|p| p == "network");
        if network != !self.network.is_empty() {
            return bad("the network permission and the \"network\" hosts go together".into());
        }
        if self.network.len() > 20 || self.network.iter().any(|h| !valid_host(h)) {
            return bad("network takes up to 20 host names like api.example.com or *.example.com".into());
        }
        // Texts: the plugin's own keys and replacements for Glyph's.
        let own = format!("{}.", self.id);
        for (lang, texts) in &self.strings {
            if !valid_lang(lang) {
                return bad(format!("strings: \"{lang}\" is not a two-letter language code"));
            }
            if texts.len() > 2000 {
                return bad("strings: at most 2000 texts per language".into());
            }
            for (key, text) in texts {
                if !valid_text_key(key) {
                    return bad(format!("strings: \"{key}\" is not a valid text key"));
                }
                if !key.starts_with(&own) && protected(key) {
                    return bad(format!("strings: \"{key}\" can't be replaced (plugin warnings, account and assistant texts stay Glyph's)"));
                }
                if text.chars().count() > 500 {
                    return bad(format!("strings: \"{key}\" is longer than 500 characters"));
                }
            }
        }
        if self.hide.len() > 200 {
            return bad("hide takes up to 200 rows".into());
        }
        for key in &self.hide {
            if !valid_text_key(key) || protected(key) {
                return bad(format!("hide: \"{key}\" can't be hidden"));
            }
        }
        // Settings.
        if self.settings.len() > 20 {
            return bad("settings: at most 20 sections".into());
        }
        let mut ids: Vec<&str> = Vec::new();
        for s in &self.settings {
            if !valid_key(&s.id) || ids.contains(&s.id.as_str()) {
                return bad(format!("settings: section id \"{}\" must be unique, 1-40 of a-z, 0-9, _ and -", s.id));
            }
            ids.push(&s.id);
            if !PAGES.contains(&s.page.as_str()) {
                // A page this build does not know: newer Glyph.
                return Err(Invalid::NeedsNewerApp);
            }
            if !s.after.is_empty() && !valid_text_key(&s.after) {
                return bad(format!("settings: \"{}\" is not a row's label key", s.after));
            }
            if s.title.longest() > 60 || !s.title.langs_ok() {
                return bad(format!("settings: the title of \"{}\" is too long (60) or has a bad language code", s.id));
            }
            if s.items.is_empty() || s.items.len() > 40 {
                return bad(format!("settings: section \"{}\" needs 1-40 items", s.id));
            }
            for i in &s.items {
                self.validate_item(&s.id, i)?;
                if i.secret && s.page != "own" { return bad("credentials must be on the plugin own page".into()); }
            }
        }
        let mut item_ids: Vec<&str> = self.settings.iter().flat_map(|s| s.items.iter().map(|i| i.id.as_str())).collect();
        let n = item_ids.len();
        item_ids.sort_unstable();
        item_ids.dedup();
        if item_ids.len() != n {
            return bad("settings: item ids must be unique across all sections".into());
        }
        if self.menus.len() > 10 {
            return bad("menus: at most 10 entries".into());
        }
        for (i, m) in self.menus.iter().enumerate() {
            if !valid_key(&m.id) || self.menus[..i].iter().any(|o| o.id == m.id) {
                return bad(format!("menus: id \"{}\" must be unique, 1-40 of a-z, 0-9, _ and -", m.id));
            }
            if m.menu != "song" {
                return Err(Invalid::NeedsNewerApp);
            }
            if m.label.is_empty() || m.label.longest() > 40 || !m.label.langs_ok() {
                return bad(format!("menus: \"{}\" needs a label of 1-40 characters", m.id));
            }
        }
        Ok(())
    }

    fn validate_item(&self, section: &str, i: &SettingItem) -> Result<(), Invalid> {
        let bad = |s: String| Err(Invalid::Bad(format!("settings: {section}.{}: {s}", i.id)));
        if !valid_key(&i.id) {
            return bad("id must be 1-40 of a-z, 0-9, _ and -".into());
        }
        if !ITEM_TYPES.contains(&i.kind.as_str()) {
            return Err(Invalid::NeedsNewerApp);
        }
        if i.label.longest() > 60 || i.desc.longest() > 300 || i.placeholder.longest() > 60 || i.caption.longest() > 40 {
            return bad("label may have 60 characters, desc 300, placeholder 60, caption 40".into());
        }
        if ![&i.label, &i.desc, &i.placeholder, &i.caption].iter().all(|t| t.langs_ok()) || i.options.iter().any(|o| !o.label.langs_ok()) {
            return bad("texts take two-letter language codes".into());
        }
        if i.kind != "note" && i.label.is_empty() {
            return bad("needs a label".into());
        }
        let d = i.default.as_ref();
        if (i.secret || i.required) && (i.kind != "text" || !i.secret || i.default.is_some()) {
            return bad("credentials must be secret text rows without defaults; required needs secret".into());
        }
        if i.secret != i.credential.is_some() { return bad("secret fields require a credential destination".into()); }
        if let Some(c) = &i.credential {
            if !valid_host(&c.host) || c.host.starts_with("*.") || !host_allowed(&self.network, &c.host)
                || !self.grants("network") || !matches!(c.location.as_str(), "query" | "headers")
                || c.name.is_empty() || c.name.len() > 100
                || !c.name.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)) {
                return bad("credential needs an exact declared host, query or headers location and field name".into());
            }
            let url = reqwest::Url::parse(&format!("https://{}{}", c.host, c.path));
            if !c.path.starts_with('/') || c.path.len() > 200 || !url.is_ok_and(|u| u.host_str() == Some(c.host.as_str())
                && u.path() == c.path && u.query().is_none() && u.fragment().is_none()) {
                return bad("credential path must be an exact normalized path without query or fragment".into());
            }
        }
        if i.required && self.main.is_none() {
            return bad("required credentials need code".into());
        }
        match i.kind.as_str() {
            "toggle" if d.is_some_and(|v| !v.is_boolean()) => bad("default must be true or false".into()),
            "slider" => {
                let (min, max) = (i.min.unwrap_or(0.0), i.max.unwrap_or(100.0));
                if !(min.is_finite() && max.is_finite() && min < max) || i.step.is_some_and(|s| !(s > 0.0 && s.is_finite())) {
                    return bad("needs min < max and a positive step".into());
                }
                if d.is_some_and(|v| v.as_f64().is_none_or(|x| x < min || x > max)) {
                    return bad("default must be a number between min and max".into());
                }
                if i.unit.chars().count() > 8 {
                    return bad("unit may have 8 characters".into());
                }
                Ok(())
            }
            "select" => {
                if i.options.is_empty() || i.options.len() > 50 || i.options.iter().any(|o| o.value.is_empty() || o.value.len() > 60 || o.label.is_empty() || o.label.longest() > 60) {
                    return bad("needs 1-50 options, each with a value and a label".into());
                }
                if d.is_some_and(|v| !v.as_str().is_some_and(|s| i.options.iter().any(|o| o.value == s))) {
                    return bad("default must be one of the option values".into());
                }
                Ok(())
            }
            "text" if d.is_some_and(|v| !v.as_str().is_some_and(|s| s.chars().count() <= 500)) => bad("default must be text (500 characters at most)".into()),
            "link" if !i.url.starts_with("https://") || i.url.len() > 500 => bad("needs an https url".into()),
            _ => Ok(()),
        }
    }

    /// Without code: safe to install from unchecked repos.
    pub fn data_only(&self) -> bool {
        self.main.is_none()
    }

    /// The code may use `permission`.
    pub fn grants(&self, permission: &str) -> bool {
        self.permissions.iter().any(|p| p == permission)
    }

    /// Permissions and hosts `self` asks for beyond `old` (an update that
    /// needs the user's consent again).
    pub fn new_permissions(&self, old: &Manifest) -> Vec<String> {
        let mut out: Vec<String> = self.permissions.iter().filter(|p| !old.grants(p)).cloned().collect();
        out.extend(self.network.iter().filter(|h| !old.network.contains(h)).map(|h| format!("network:{h}")));
        if self.main.is_some() && old.main.is_none() {
            out.insert(0, "code".into());
        }
        out
    }

    /// Settings sections of `page` (one of [`PAGES`]).
    pub fn sections_on<'a>(&'a self, page: &'a str) -> impl Iterator<Item = &'a SettingsSection> + 'a {
        self.settings.iter().filter(move |s| s.page == page)
    }

    /// Every settings item, by id.
    pub fn item(&self, id: &str) -> Option<&SettingItem> {
        self.settings.iter().flat_map(|s| s.items.iter()).find(|i| i.id == id)
    }

    pub fn missing_credentials(&self) -> Vec<String> {
        self.settings.iter().flat_map(|s| &s.items)
            .filter(|i| i.required && crate::plugin_credentials::load(&self.id, i).is_none())
            .map(|i| i.id.clone()).collect()
    }

    /// Colours of a valid theme plugin.
    pub fn theme_colors(&self) -> Option<ThemeColors> {
        self.theme.as_ref().and_then(|t| t.colors().ok())
    }
}

/// `a` is a higher semantic version than `b`.
pub fn is_newer(a: &str, b: &str) -> bool {
    matches!((semver::Version::parse(a), semver::Version::parse(b)), (Ok(a), Ok(b)) if a > b)
}

pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// ------------------------------------------------------------------ catalog

/// One installable plugin from the catalog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Listing {
    /// `owner/name` on GitHub.
    pub repo: String,
    /// Folder of the manifest inside the repo; empty = root.
    #[serde(default)]
    pub path: String,
    /// Commit (checked) or `HEAD` (community).
    pub rev: String,
    /// Listed in `glyphmusic/plugins` (checked by CI and a maintainer).
    #[serde(default)]
    pub checked: bool,
    /// GitHub stars of the repo; `None` for plugins in a folder of a shared
    /// repo (the repo's stars are not theirs).
    #[serde(default)]
    pub stars: Option<u32>,
    /// Last push to the repo (ISO 8601), for "recently updated".
    #[serde(default)]
    pub updated: Option<String>,
    pub manifest: Manifest,
}

impl Listing {
    pub fn repo_url(&self) -> String {
        if self.path.is_empty() {
            format!("https://github.com/{}", self.repo)
        } else {
            format!("https://github.com/{}/tree/{}/{}", self.repo, self.rev, self.path)
        }
    }

    /// May be installed: valid for this Glyph, and code only when checked.
    pub fn installable(&self) -> Result<(), Invalid> {
        self.manifest.validate(app_version())?;
        if !self.checked && !self.manifest.data_only() {
            return Err(Invalid::Bad("only checked plugins may run code".into()));
        }
        Ok(())
    }
}

/// `index.json` of `glyphmusic/plugins`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Index {
    #[serde(default)]
    pub plugins: Vec<IndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub repo: String,
    #[serde(default)]
    pub path: String,
    pub rev: String,
    /// Filled in daily by the index CI (repo stars and last push).
    #[serde(default)]
    pub stars: Option<u32>,
    #[serde(default)]
    pub updated: Option<String>,
    pub manifest: Manifest,
}

/// A repo found by topic (the search API's fields we use).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicRepo {
    pub full_name: String,
    #[serde(default)]
    pub stargazers_count: u32,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub pushed_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Catalog {
    /// Unix seconds of the last refresh.
    pub fetched_at: u64,
    pub checked: Vec<Listing>,
    pub community: Vec<Listing>,
    /// ETag of the last topic search: an unchanged answer (304) does not
    /// count against GitHub's rate limit.
    #[serde(default)]
    pub search_etag: Option<String>,
    #[serde(default)]
    pub topic_repos: Vec<TopicRepo>,
    /// The last refresh could not reach one of the sources (shown, the
    /// cached part stays).
    #[serde(default)]
    pub partial: bool,
}

impl Catalog {
    /// Every listing, checked first.
    pub fn all(&self) -> impl Iterator<Item = &Listing> {
        self.checked.iter().chain(self.community.iter())
    }

    pub fn find(&self, id: &str) -> Option<&Listing> {
        self.all().find(|l| l.manifest.id == id)
    }
}

pub fn plugins_dir() -> PathBuf {
    paths::data_dir().join("plugins")
}

fn catalog_path() -> PathBuf {
    plugins_dir().join("catalog.json")
}

/// The cached catalog, however old.
pub fn cached_catalog() -> Option<Catalog> {
    paths::read_json(&catalog_path())
}

/// The catalog: cached while younger than an hour, unless `force`.
pub async fn catalog(http: &reqwest::Client, force: bool) -> Result<Catalog> {
    let cached = cached_catalog();
    if let Some(c) = &cached {
        if !force && !c.partial && paths::now_unix().saturating_sub(c.fetched_at) < CATALOG_TTL_S {
            return Ok(c.clone());
        }
    }
    let prev = cached.unwrap_or_default();
    let (index, search) = futures::join!(fetch_index(http), search_topic(http, prev.search_etag.as_deref()));

    let mut partial = false;
    let mut checked = match index {
        Ok(i) => listings_from_index(i),
        Err(e) => {
            tracing::warn!("plugin index: {e:#}");
            partial = true;
            prev.checked.clone()
        }
    };
    let (topic_repos, etag, refetch) = match search {
        Ok(Search::Fresh(repos, etag)) => (repos, etag, true),
        Ok(Search::Unchanged) => (prev.topic_repos.clone(), prev.search_etag.clone(), false),
        Err(e) => {
            tracing::warn!("plugin topic search: {e:#}");
            partial = true;
            (prev.topic_repos.clone(), prev.search_etag.clone(), false)
        }
    };
    // The topic search is fresher than the index's daily stars.
    for l in checked.iter_mut().filter(|l| l.path.is_empty()) {
        if let Some(r) = topic_repos.iter().find(|r| r.full_name.eq_ignore_ascii_case(&l.repo)) {
            l.stars = Some(r.stargazers_count);
            if !r.pushed_at.is_empty() {
                l.updated = Some(r.pushed_at.clone());
            }
        }
    }
    let community = if refetch || prev.community.is_empty() || force {
        community_listings(http, &topic_repos, &checked).await
    } else {
        prev.community.into_iter().filter(|l| !checked.iter().any(|c| c.manifest.id == l.manifest.id || c.repo.eq_ignore_ascii_case(&l.repo))).collect()
    };
    if checked.is_empty() && community.is_empty() && partial {
        bail!("plugin catalog unreachable");
    }
    let c = Catalog { fetched_at: paths::now_unix(), checked, community, search_etag: etag, topic_repos, partial };
    if let Err(e) = paths::write_json(&catalog_path(), &c) {
        tracing::warn!("plugin catalog cache: {e:#}");
    }
    Ok(c)
}

async fn fetch_index(http: &reqwest::Client) -> Result<Index> {
    let r = http.get(INDEX_URL).send().await?.error_for_status()?;
    let bytes = r.bytes().await?;
    serde_json::from_slice(&bytes).context("index.json")
}

/// Checked listings, dropping malformed entries and duplicate ids.
fn listings_from_index(index: Index) -> Vec<Listing> {
    let mut out: Vec<Listing> = Vec::new();
    for e in index.plugins {
        if !valid_repo(&e.repo) || !valid_path(&e.path) || !valid_id(&e.manifest.id) || out.iter().any(|l| l.manifest.id == e.manifest.id) {
            continue;
        }
        let own_repo = e.path.is_empty();
        out.push(Listing {
            repo: e.repo,
            path: e.path,
            rev: e.rev,
            checked: true,
            stars: e.stars.filter(|_| own_repo),
            updated: e.updated,
            manifest: e.manifest,
        });
    }
    out.sort_by(|a, b| a.manifest.name.to_lowercase().cmp(&b.manifest.name.to_lowercase()));
    out
}

enum Search {
    Fresh(Vec<TopicRepo>, Option<String>),
    Unchanged,
}

async fn search_topic(http: &reqwest::Client, etag: Option<&str>) -> Result<Search> {
    #[derive(Deserialize)]
    struct Page {
        items: Vec<TopicRepo>,
    }
    let mut req = http.get(SEARCH_URL).header("accept", "application/vnd.github+json").header("x-github-api-version", "2022-11-28");
    if let Some(e) = etag {
        req = req.header("if-none-match", e);
    }
    let r = req.send().await?;
    if r.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(Search::Unchanged);
    }
    if r.status() == reqwest::StatusCode::FORBIDDEN || r.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        bail!("GitHub rate limit reached");
    }
    let r = r.error_for_status()?;
    let etag = r.headers().get("etag").and_then(|v| v.to_str().ok()).map(str::to_string);
    let page: Page = serde_json::from_slice(&r.bytes().await?).context("topic search")?;
    let repos = page.items.into_iter().filter(|r| !r.archived && valid_repo(&r.full_name)).collect();
    Ok(Search::Fresh(repos, etag))
}

/// Fetches the manifest of each topic repo not already checked; repos
/// without a valid manifest are left out.
async fn community_listings(http: &reqwest::Client, repos: &[TopicRepo], checked: &[Listing]) -> Vec<Listing> {
    let todo: Vec<TopicRepo> = repos
        .iter()
        .filter(|r| !checked.iter().any(|c| c.repo.eq_ignore_ascii_case(&r.full_name)))
        .take(COMMUNITY_MAX)
        .cloned()
        .collect();
    let mut found: Vec<Listing> = futures::stream::iter(todo)
        .map(|r| async move {
            let m = fetch_manifest(http, &r.full_name, "HEAD", "").await.ok()?;
            let updated = (!r.pushed_at.is_empty()).then(|| r.pushed_at.clone());
            Some(Listing { repo: r.full_name, path: String::new(), rev: "HEAD".into(), checked: false, stars: Some(r.stargazers_count), updated, manifest: m })
        })
        .buffer_unordered(8)
        .filter_map(|l| async move { l })
        .collect()
        .await;
    // Ids are unique: checked ones win, then the most starred repo.
    found.sort_by(|a, b| b.stars.unwrap_or(0).cmp(&a.stars.unwrap_or(0)).then_with(|| a.repo.cmp(&b.repo)));
    let mut out: Vec<Listing> = Vec::new();
    for l in found {
        if valid_id(&l.manifest.id) && !checked.iter().chain(out.iter()).any(|c| c.manifest.id == l.manifest.id) {
            out.push(l);
        }
    }
    out
}

/// `glyph-plugin.json` of `repo` at `rev` (raw.githubusercontent.com has no
/// API rate limit).
pub async fn fetch_manifest(http: &reqwest::Client, repo: &str, rev: &str, path: &str) -> Result<Manifest> {
    if !valid_repo(repo) || !valid_path(path) || !rev.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_./".contains(&b)) {
        bail!("invalid plugin location");
    }
    let file = if path.is_empty() { MANIFEST_FILE.to_string() } else { format!("{path}/{MANIFEST_FILE}") };
    let url = format!("https://raw.githubusercontent.com/{repo}/{rev}/{file}");
    let r = http.get(url).send().await?.error_for_status()?;
    let bytes = r.bytes().await?;
    if bytes.len() > MAX_MANIFEST {
        bail!("manifest too large");
    }
    serde_json::from_slice(&bytes).context(MANIFEST_FILE)
}

// ------------------------------------------------------------------ search

/// How the catalog is ordered (Settings -> Plugins, glyph.my/plugins).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    /// Most GitHub stars first.
    #[default]
    Stars,
    /// Most recently pushed first.
    Updated,
    Name,
}

/// Fuzzy match of `query` in `text`: `None` unless every query character
/// appears in order (case-insensitive). Higher is better: characters at word
/// starts and runs of consecutive characters count more, gaps less, an exact
/// substring (above all at the start) most.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    let q: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Some(0);
    }
    let lower = text.to_lowercase();
    let t: Vec<char> = lower.chars().collect();
    let (mut score, mut qi, mut prev) = (0i32, 0usize, None::<usize>);
    for (i, c) in t.iter().enumerate() {
        if qi == q.len() {
            break;
        }
        if *c != q[qi] {
            continue;
        }
        score += 1;
        if i == 0 || !t[i - 1].is_alphanumeric() {
            score += 8;
        }
        if let Some(p) = prev {
            score += if p + 1 == i { 5 } else { -((i - p - 1) as i32).min(3) };
        }
        prev = Some(i);
        qi += 1;
    }
    if qi < q.len() {
        return None;
    }
    let whole = query.trim().to_lowercase();
    if lower.starts_with(&whole) {
        score += 30;
    } else if lower.contains(&whole) {
        score += 20;
    }
    Some(score)
}

/// Best match of `query` in a listing: name first, then id, author and
/// description.
pub fn search_score(l: &Listing, query: &str) -> Option<i32> {
    let m = &l.manifest;
    [
        fuzzy_score(query, &m.name).map(|s| s + 15),
        fuzzy_score(query, &m.id).map(|s| s + 10),
        fuzzy_score(query, &m.author),
        fuzzy_score(query, &m.description).map(|s| s - 5),
    ]
    .into_iter()
    .flatten()
    .max()
}

/// `items` matching `query` (all for an empty one), best matches first while
/// searching, otherwise (and among equal matches) in `sort` order.
pub fn filter_sort<'a>(items: &'a [Listing], query: &str, sort: Sort) -> Vec<&'a Listing> {
    let searching = !query.trim().is_empty();
    let mut hits: Vec<(i32, &Listing)> = items
        .iter()
        .filter_map(|l| if searching { search_score(l, query).map(|s| (s, l)) } else { Some((0, l)) })
        .collect();
    hits.sort_by(|(sa, a), (sb, b)| {
        sb.cmp(sa)
            .then_with(|| match sort {
                Sort::Stars => b.stars.unwrap_or(0).cmp(&a.stars.unwrap_or(0)),
                // ISO 8601 strings order like the times they stand for.
                Sort::Updated => b.updated.cmp(&a.updated),
                Sort::Name => std::cmp::Ordering::Equal,
            })
            .then_with(|| a.manifest.name.to_lowercase().cmp(&b.manifest.name.to_lowercase()))
    });
    hits.into_iter().map(|(_, l)| l).collect()
}

// ------------------------------------------------------------------ installed

/// Where an installed plugin came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Origin {
    pub repo: String,
    #[serde(default)]
    pub path: String,
    pub rev: String,
    #[serde(default)]
    pub checked: bool,
    pub installed_at: u64,
}

/// `repo` of plugins put into the plugins folder by hand (no
/// `origin.json`): plugin makers testing their work before publishing.
pub const LOCAL_REPO: &str = "local";

impl Origin {
    fn local() -> Self {
        Self { repo: LOCAL_REPO.into(), path: String::new(), rev: "local".into(), checked: false, installed_at: 0 }
    }

    /// Put into the plugins folder by hand, not installed from GitHub.
    pub fn is_local(&self) -> bool {
        self.repo == LOCAL_REPO
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    pub manifest: Manifest,
    pub origin: Origin,
}

impl Installed {
    /// The catalog's listing of this plugin when it is newer (same repo).
    pub fn update_in<'a>(&self, catalog: &'a Catalog) -> Option<&'a Listing> {
        let l = catalog.find(&self.manifest.id)?;
        if !l.repo.eq_ignore_ascii_case(&self.origin.repo) {
            return None;
        }
        (is_newer(&l.manifest.version, &self.manifest.version) && l.installable().is_ok()).then_some(l)
    }
}

/// Installed plugins that are still valid for this Glyph, sorted by name.
pub fn installed() -> Vec<Installed> {
    let Ok(dir) = std::fs::read_dir(plugins_dir()) else { return Vec::new() };
    let mut out: Vec<Installed> = dir
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let p = e.path();
            let manifest: Manifest = paths::read_json(&p.join(MANIFEST_FILE))?;
            // No origin.json: copied in by hand to try it out (local).
            let origin: Origin = paths::read_json(&p.join("origin.json")).unwrap_or_else(Origin::local);
            // The folder name is the id; anything else was tampered with.
            let ok = e.file_name().to_str() == Some(manifest.id.as_str()) && manifest.validate(app_version()).is_ok();
            ok.then_some(Installed { manifest, origin })
        })
        .collect();
    out.sort_by(|a, b| a.manifest.name.to_lowercase().cmp(&b.manifest.name.to_lowercase()));
    out
}

impl Installed {
    /// The plugin's WebAssembly module, if it has code: `code.wasm` for
    /// installed plugins, the manifest's `main` for local ones.
    pub fn code_path(&self) -> Option<PathBuf> {
        let main = self.manifest.main.as_ref()?;
        let dir = plugins_dir().join(&self.manifest.id);
        Some(if self.origin.is_local() { dir.join(main) } else { dir.join(CODE_FILE) })
    }

    pub fn dir(&self) -> PathBuf {
        plugins_dir().join(&self.manifest.id)
    }
}

/// Checks that `bytes` look like a WebAssembly module of acceptable size.
pub fn check_code(bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_CODE {
        bail!("plugin code is larger than {} MiB", MAX_CODE / (1024 * 1024));
    }
    if !bytes.starts_with(b"\0asm") {
        bail!("plugin code is not a WebAssembly module");
    }
    Ok(())
}

/// Downloads the code of `listing` (its `main`, at the listed commit);
/// `None` for plugins without code.
pub async fn fetch_code(http: &reqwest::Client, listing: &Listing) -> Result<Option<Vec<u8>>> {
    let Some(main) = &listing.manifest.main else { return Ok(None) };
    if !valid_repo(&listing.repo) || !valid_path(&listing.path) || !valid_path(main) || !listing.rev.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_./".contains(&b)) {
        bail!("invalid plugin location");
    }
    let file = if listing.path.is_empty() { main.clone() } else { format!("{}/{main}", listing.path) };
    let url = format!("https://raw.githubusercontent.com/{}/{}/{file}", listing.repo, listing.rev);
    let r = http.get(url).send().await?.error_for_status()?;
    if r.content_length().is_some_and(|n| n as usize > MAX_CODE) {
        bail!("plugin code is larger than {} MiB", MAX_CODE / (1024 * 1024));
    }
    let mut body = r.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = body.next().await {
        bytes.extend_from_slice(&chunk?);
        if bytes.len() > MAX_CODE {
            bail!("plugin code is larger than {} MiB", MAX_CODE / (1024 * 1024));
        }
    }
    check_code(&bytes)?;
    Ok(Some(bytes))
}

/// Installs (or updates) `listing` with its `code` (from [`fetch_code`]).
/// Another repo's plugin with the same id is refused rather than replaced.
pub fn install(listing: &Listing, code: Option<&[u8]>) -> Result<Installed> {
    listing.installable().map_err(|e| anyhow!("{}: {e}", listing.manifest.id))?;
    let id = &listing.manifest.id;
    let dir = plugins_dir().join(id);
    if let Some(origin) = paths::read_json::<Origin>(&dir.join("origin.json")) {
        if !origin.repo.eq_ignore_ascii_case(&listing.repo) {
            bail!("another plugin ({}) already uses the id {id}", origin.repo);
        }
    }
    match (&listing.manifest.main, code) {
        (Some(_), Some(code)) => check_code(code)?,
        (Some(_), None) => bail!("{id}: the plugin's code is missing"),
        (None, _) => {}
    }
    // An update cannot reuse a key under a changed destination declaration.
    if let Some(old) = paths::read_json::<Manifest>(&dir.join(MANIFEST_FILE)).filter(|m| m.id == *id) {
        for item in old.settings.iter().flat_map(|s| &s.items).filter(|i| i.secret && i.credential.is_some()) {
            if listing.manifest.item(&item.id).is_none_or(|new| !new.secret || new.credential != item.credential) {
                crate::plugin_credentials::store(id, item, "")?;
            }
        }
    }
    let origin = Origin {
        repo: listing.repo.clone(),
        path: listing.path.clone(),
        rev: listing.rev.clone(),
        checked: listing.checked,
        installed_at: paths::now_unix(),
    };
    std::fs::create_dir_all(&dir)?;
    let code_file = dir.join(CODE_FILE);
    match code.filter(|_| listing.manifest.main.is_some()) {
        Some(code) => {
            // Written beside, then moved: a running copy never reads half a file.
            let tmp = dir.join("code.wasm.part");
            std::fs::write(&tmp, code)?;
            std::fs::rename(&tmp, &code_file)?;
        }
        None if code_file.exists() => std::fs::remove_file(&code_file)?,
        None => {}
    }
    paths::write_json(&dir.join(MANIFEST_FILE), &listing.manifest)?;
    paths::write_json(&dir.join("origin.json"), &origin)?;
    Ok(Installed { manifest: listing.manifest.clone(), origin })
}

/// Values of a plugin's settings items (`values.json`), by item id.
pub fn load_values(id: &str) -> serde_json::Map<String, serde_json::Value> {
    if !valid_id(id) {
        return Default::default();
    }
    paths::read_json(&plugins_dir().join(id).join("values.json")).unwrap_or_default()
}

pub fn save_values(id: &str, values: &serde_json::Map<String, serde_json::Value>) -> Result<()> {
    if !valid_id(id) {
        bail!("invalid plugin id");
    }
    paths::write_json(&plugins_dir().join(id).join("values.json"), values)
}

/// The value of `item` in `values`, else its default, else the type's
/// empty value.
pub fn item_value(item: &SettingItem, values: &serde_json::Map<String, serde_json::Value>) -> serde_json::Value {
    if let Some(v) = values.get(&item.id) {
        return v.clone();
    }
    if let Some(d) = &item.default {
        return d.clone();
    }
    match item.kind.as_str() {
        "toggle" => serde_json::Value::Bool(false),
        "slider" => serde_json::json!(item.min.unwrap_or(0.0)),
        "select" => item.options.first().map(|o| serde_json::Value::String(o.value.clone())).unwrap_or_default(),
        "text" => serde_json::Value::String(String::new()),
        _ => serde_json::Value::Null,
    }
}

/// `value` fits `item` (a plugin or the UI setting it); sliders are
/// clamped and snapped to their step.
pub fn coerce_value(item: &SettingItem, value: &serde_json::Value) -> Option<serde_json::Value> {
    match item.kind.as_str() {
        "toggle" => value.as_bool().map(serde_json::Value::Bool),
        "slider" => {
            let (min, max) = (item.min.unwrap_or(0.0), item.max.unwrap_or(100.0));
            let mut x = value.as_f64()?.clamp(min, max);
            if let Some(step) = item.step {
                x = (min + ((x - min) / step).round() * step).clamp(min, max);
            }
            Some(serde_json::json!(x))
        }
        "select" => value.as_str().filter(|s| item.options.iter().any(|o| o.value == *s)).map(|s| serde_json::Value::String(s.into())),
        "text" => value.as_str().map(|s| serde_json::Value::String(s.chars().take(500).collect())),
        _ => None,
    }
}

/// Settings rows hidden by `plugins` (label keys).
pub fn hidden_rows(plugins: &[&Manifest]) -> Vec<String> {
    let mut out: Vec<String> = plugins.iter().flat_map(|m| m.hide.iter().cloned()).collect();
    out.sort();
    out.dedup();
    out
}

/// A plugin's folder (`None` for an invalid id).
pub fn plugin_dir(id: &str) -> Option<PathBuf> {
    valid_id(id).then(|| plugins_dir().join(id))
}

pub fn uninstall(id: &str) -> Result<()> {
    if !valid_id(id) {
        bail!("invalid plugin id");
    }
    let dir = plugins_dir().join(id);
    if let Some(m) = paths::read_json::<Manifest>(&dir.join(MANIFEST_FILE)).filter(|m| m.id == id) {
        for i in m.settings.iter().flat_map(|s| &s.items).filter(|i| i.secret) {
            crate::plugin_credentials::store(id, i, "")?;
        }
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

/// Theme id under which a theme plugin appears in the config.
pub fn theme_id(plugin_id: &str) -> String {
    format!("plugin:{plugin_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(id: &str, version: &str) -> Manifest {
        Manifest {
            schema: 1,
            id: id.into(),
            name: "Sunset".into(),
            version: version.into(),
            author: "glyph".into(),
            description: String::new(),
            license: "MIT".into(),
            min_app: None,
            kind: "theme".into(),
            theme: Some(ThemeSpec { accent: "#ff8a3d".into(), glow: Some(["#3a1206".into(), "#ff6516".into(), "#ffc04a".into()]), base: Some("#050302".into()) }),
            main: None,
            permissions: Vec::new(),
            network: Vec::new(),
            strings: BTreeMap::new(),
            hide: Vec::new(),
            settings: Vec::new(),
            menus: Vec::new(),
        }
    }

    fn extension(id: &str) -> Manifest {
        let json = format!(
            r##"{{"schema":2,"id":"{id}","name":"Scrobbler","version":"1.0.0","kind":"extension","main":"plugin.wasm",
            "permissions":["playback","network"],"network":["ws.audioscrobbler.com"],
            "strings":{{"en":{{"{id}.hello":"Hello","sp.crossfade":"Blend"}},"de":{{"{id}.hello":"Hallo"}}}},
            "hide":["sp.radio"],
            "settings":[{{"id":"main","title":{{"en":"Scrobbling","de":"Scrobbeln"}},"items":[
              {{"id":"enabled","type":"toggle","label":"Scrobble","default":true}},
              {{"id":"user","type":"text","label":"User"}},
              {{"id":"percent","type":"slider","label":"After","min":10,"max":100,"step":5,"default":50,"unit":"%"}},
              {{"id":"mode","type":"select","label":"Mode","options":[{{"value":"a","label":"A"}},{{"value":"b","label":"B"}}],"default":"b"}},
              {{"id":"login","type":"button","label":"Connect"}},
              {{"id":"info","type":"note","desc":"Text"}},
              {{"id":"site","type":"link","label":"Website","url":"https://example.com"}}]}},
              {{"id":"quick","page":"playback","after":"sp.crossfade","items":[{{"id":"quick_on","type":"toggle","label":"On"}}]}}],
            "menus":[{{"id":"love","label":"Love on Last.fm"}}]}}"##
        );
        serde_json::from_str(&json).unwrap()
    }

    fn listing(m: Manifest, repo: &str, checked: bool) -> Listing {
        Listing { repo: repo.into(), path: String::new(), rev: "HEAD".into(), checked, stars: None, updated: None, manifest: m }
    }

    #[test]
    fn ids_and_repos() {
        assert!(valid_id("sunset"));
        assert!(valid_id("lo-fi-2"));
        assert!(!valid_id("Sunset"));
        assert!(!valid_id("-a"));
        assert!(!valid_id("a--b"));
        assert!(!valid_id("x"));
        assert!(!valid_id("../x"));
        assert!(valid_repo("glyphmusic/plugins"));
        assert!(!valid_repo("glyphmusic"));
        assert!(!valid_repo("a/b/c"));
        assert!(!valid_repo("../x"));
        assert!(valid_path("themes/sunset"));
        assert!(!valid_path("themes/../x"));
    }

    #[test]
    fn manifest_parses_and_validates() {
        let json = r##"{"id":"sunset","name":"Sunset","version":"1.0.0","kind":"theme","theme":{"accent":"#ff8a3d"}}"##;
        let m: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.schema, 1);
        assert_eq!(m.validate("0.6.1"), Ok(()));
        let c = m.theme_colors().unwrap();
        assert_eq!(c.accent, 0xff8a3d);
        assert_eq!(c.base, DEFAULT_BASE);
        assert_eq!(c.glow, None);
    }

    #[test]
    fn rejects_bad_manifests() {
        let mut m = theme("sunset", "1.0");
        assert!(matches!(m.validate("0.6.1"), Err(Invalid::Bad(_))));
        m.version = "1.0.0".into();
        m.theme.as_mut().unwrap().base = Some("#ffffff".into());
        assert!(matches!(m.validate("0.6.1"), Err(Invalid::Bad(_))));
        m.theme.as_mut().unwrap().base = None;
        m.theme.as_mut().unwrap().accent = "red".into();
        assert!(matches!(m.validate("0.6.1"), Err(Invalid::Bad(_))));
        let mut m = theme("sunset", "1.0.0");
        m.theme = None;
        assert!(matches!(m.validate("0.6.1"), Err(Invalid::Bad(_))));
        let m = theme(TEMPLATE_ID, "1.0.0");
        assert!(matches!(m.validate("0.6.1"), Err(Invalid::Bad(_))));
    }

    #[test]
    fn newer_plugins_wait_for_a_newer_app() {
        let mut m = theme("sunset", "1.0.0");
        m.min_app = Some("9.0.0".into());
        assert_eq!(m.validate("0.6.1"), Err(Invalid::NeedsNewerApp));
        let mut m = theme("sunset", "1.0.0");
        m.schema = SCHEMA + 1;
        assert_eq!(m.validate("0.6.1"), Err(Invalid::NeedsNewerApp));
        let mut m = theme("sunset", "1.0.0");
        m.kind = "lyrics".into();
        assert_eq!(m.validate("0.6.1"), Err(Invalid::NeedsNewerApp));
    }

    #[test]
    fn unchecked_code_is_not_installable() {
        let mut m = theme("sunset", "1.0.0");
        assert!(listing(m.clone(), "a/b", false).installable().is_ok());
        // Code is refused when unchecked.
        m.kind = "wasm".into();
        m.main = Some("x.wasm".into());
        assert!(!m.data_only());
        assert!(listing(m, "a/b", false).installable().is_err());
        let e = extension("scrobbler");
        assert!(listing(e.clone(), "a/b", false).installable().is_err());
        assert!(listing(e, "a/b", true).installable().is_ok());
    }

    #[test]
    fn extensions_validate() {
        let e = extension("scrobbler");
        assert_eq!(e.validate("0.7.0"), Ok(()));
        assert!(e.grants("playback") && !e.grants("library"));
        assert_eq!(e.sections_on("own").count(), 1);
        assert_eq!(e.sections_on("playback").count(), 1);
        assert_eq!(e.settings[0].title.get("de"), "Scrobbeln");
        assert_eq!(e.settings[0].title.get("fr"), "Scrobbling");
        assert_eq!(e.item("mode").unwrap().options.len(), 2);
        // Schema 1 knows no extensions.
        let mut m = e.clone();
        m.schema = 1;
        assert!(matches!(m.validate("0.7.0"), Err(Invalid::Bad(_))));
        // Network hosts and the permission go together.
        let mut m = e.clone();
        m.network.clear();
        assert!(matches!(m.validate("0.7.0"), Err(Invalid::Bad(_))));
        let mut m = e.clone();
        m.network = vec!["https://x.com/".into()];
        assert!(matches!(m.validate("0.7.0"), Err(Invalid::Bad(_))));
        // Unknown permissions, pages and item types come from a newer API.
        let mut m = e.clone();
        m.permissions.push("camera".into());
        assert_eq!(m.validate("0.7.0"), Err(Invalid::NeedsNewerApp));
        let mut m = e.clone();
        m.settings[1].page = "lyrics".into();
        assert_eq!(m.validate("0.7.0"), Err(Invalid::NeedsNewerApp));
        // The plugin warnings can be neither replaced nor hidden.
        let mut m = e.clone();
        m.strings.get_mut("en").unwrap().insert("plugins.warn.title".into(), "All good".into());
        assert!(matches!(m.validate("0.7.0"), Err(Invalid::Bad(_))));
        let mut m = e.clone();
        m.hide.push("plugins.auto_update".into());
        assert!(matches!(m.validate("0.7.0"), Err(Invalid::Bad(_))));
        // Settings and menus need code.
        let mut m = e.clone();
        m.main = None;
        assert!(matches!(m.validate("0.7.0"), Err(Invalid::Bad(_))));
        // Duplicate item ids, bad defaults.
        let mut m = e.clone();
        m.settings[1].items[0].id = "user".into();
        assert!(matches!(m.validate("0.7.0"), Err(Invalid::Bad(_))));
        let mut m = e.clone();
        m.settings[0].items[2].default = Some(serde_json::json!(500));
        assert!(matches!(m.validate("0.7.0"), Err(Invalid::Bad(_))));
        // Texts-only extensions are data.
        let mut m = e;
        m.main = None;
        m.settings.clear();
        m.menus.clear();
        m.permissions.clear();
        m.network.clear();
        assert_eq!(m.validate("0.7.0"), Ok(()));
        assert!(m.data_only());
        // A theme with extras must say it is an extension.
        let mut t = theme("sunset", "1.0.0");
        t.hide.push("sp.radio".into());
        assert!(matches!(t.validate("0.7.0"), Err(Invalid::Bad(_))));
    }

    #[test]
    fn hosts_and_permissions() {
        assert!(valid_host("api.example.com"));
        assert!(valid_host("*.example.com"));
        assert!(!valid_host("example"));
        assert!(!valid_host("api.example.com:443"));
        assert!(!valid_host("API.example.com"));
        let allowed = vec!["api.example.com".to_string(), "*.cdn.net".to_string()];
        assert!(host_allowed(&allowed, "api.example.com"));
        assert!(host_allowed(&allowed, "a.b.cdn.net"));
        assert!(host_allowed(&allowed, "cdn.net"));
        assert!(!host_allowed(&allowed, "evil.com"));
        assert!(!host_allowed(&allowed, "api.example.com.evil.com"));
        let old = extension("scrobbler");
        let mut new = old.clone();
        assert!(new.new_permissions(&old).is_empty());
        new.permissions.push("library".into());
        new.network.push("api.example.com".into());
        assert_eq!(new.new_permissions(&old), ["library", "network:api.example.com"]);
    }

    #[test]
    fn item_values() {
        let e = extension("scrobbler");
        let mut values = serde_json::Map::new();
        assert_eq!(item_value(e.item("enabled").unwrap(), &values), serde_json::json!(true));
        assert_eq!(item_value(e.item("user").unwrap(), &values), serde_json::json!(""));
        values.insert("mode".into(), serde_json::json!("a"));
        assert_eq!(item_value(e.item("mode").unwrap(), &values), serde_json::json!("a"));
        let slider = e.item("percent").unwrap();
        assert_eq!(coerce_value(slider, &serde_json::json!(57)), Some(serde_json::json!(55.0)));
        assert_eq!(coerce_value(slider, &serde_json::json!(1000)), Some(serde_json::json!(100.0)));
        assert_eq!(coerce_value(e.item("mode").unwrap(), &serde_json::json!("z")), None);
        assert_eq!(coerce_value(e.item("enabled").unwrap(), &serde_json::json!("yes")), None);
    }

    #[test]
    fn index_drops_bad_and_duplicate_entries() {
        let entry = |repo: &str, id: &str| IndexEntry { repo: repo.into(), path: String::new(), rev: "abc".into(), stars: None, updated: None, manifest: theme(id, "1.0.0") };
        let index = Index { plugins: vec![entry("a/b", "sunset"), entry("c/d", "sunset"), entry("bad", "other"), entry("e/f", "mint")] };
        let l = listings_from_index(index);
        assert_eq!(l.len(), 2);
        assert!(l.iter().all(|l| l.checked));
        assert_eq!(l.iter().find(|l| l.manifest.id == "sunset").unwrap().repo, "a/b");
    }

    #[test]
    fn updates_come_from_the_same_repo_only() {
        let inst = Installed {
            manifest: theme("sunset", "1.0.0"),
            origin: Origin { repo: "a/b".into(), path: String::new(), rev: "HEAD".into(), checked: false, installed_at: 0 },
        };
        let mut c = Catalog { community: vec![listing(theme("sunset", "1.1.0"), "a/b", false)], ..Default::default() };
        assert_eq!(inst.update_in(&c).unwrap().manifest.version, "1.1.0");
        c.community[0].repo = "x/y".into();
        assert!(inst.update_in(&c).is_none());
        c.community[0] = listing(theme("sunset", "1.0.0"), "a/b", false);
        assert!(inst.update_in(&c).is_none());
    }

    #[test]
    fn fuzzy_matches_in_order() {
        assert!(fuzzy_score("skr", "Sakura").is_some());
        assert!(fuzzy_score("rks", "Sakura").is_none());
        assert_eq!(fuzzy_score("", "anything"), Some(0));
        // Prefix beats substring beats scattered letters.
        let prefix = fuzzy_score("aur", "Aurora").unwrap();
        let inner = fuzzy_score("ror", "Aurora").unwrap();
        let scattered = fuzzy_score("ara", "Aurora").unwrap();
        assert!(prefix > inner && inner > scattered);
        // Word starts count: "mc" fits "Midnight Coral" better than "Smack".
        assert!(fuzzy_score("mc", "Midnight Coral").unwrap() > fuzzy_score("mc", "Smack").unwrap());
    }

    #[test]
    fn filter_and_sort() {
        let mut a = listing(theme("aurora", "1.0.0"), "a/a", true);
        a.manifest.name = "Aurora".into();
        a.stars = Some(5);
        a.updated = Some("2026-10-01T00:00:00Z".into());
        let mut b = listing(theme("sakura", "1.0.0"), "b/b", false);
        b.manifest.name = "Sakura".into();
        b.stars = Some(50);
        b.updated = Some("2026-09-01T00:00:00Z".into());
        let mut c = listing(theme("citrus", "1.0.0"), "c/c", false);
        c.manifest.name = "Citrus".into();
        c.manifest.description = "Lime and lemon".into();
        let all = vec![a, b, c];
        let ids = |v: Vec<&Listing>| v.into_iter().map(|l| l.manifest.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(filter_sort(&all, "", Sort::Stars)), ["sakura", "aurora", "citrus"]);
        assert_eq!(ids(filter_sort(&all, "", Sort::Updated)), ["aurora", "sakura", "citrus"]);
        assert_eq!(ids(filter_sort(&all, "", Sort::Name)), ["aurora", "citrus", "sakura"]);
        assert_eq!(ids(filter_sort(&all, "lemon", Sort::Stars)), ["citrus"]);
        assert_eq!(ids(filter_sort(&all, "sak", Sort::Name)), ["sakura"]);
        assert!(filter_sort(&all, "zzz", Sort::Stars).is_empty());
    }

    #[test]
    fn install_and_uninstall_roundtrip() {
        let _env = crate::paths::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("GLYPH_DATA_DIR", dir.path());
        let l = listing(theme("sunset", "1.0.0"), "a/b", false);
        install(&l, None).unwrap();
        assert_eq!(installed().len(), 1);
        // Same id from another repo is refused.
        assert!(install(&listing(theme("sunset", "2.0.0"), "x/y", true), None).is_err());
        // Update from the same repo replaces it.
        install(&listing(theme("sunset", "1.1.0"), "a/b", false), None).unwrap();
        assert_eq!(installed()[0].manifest.version, "1.1.0");
        // Code: needs a module, lands in code.wasm.
        let ext = listing(extension("scrobbler"), "c/d", true);
        assert!(install(&ext, None).is_err());
        assert!(install(&ext, Some(&b"not wasm"[..])).is_err());
        let inst = install(&ext, Some(&b"\0asm\x01\0\0\0"[..])).unwrap();
        assert!(inst.code_path().unwrap().ends_with(CODE_FILE));
        assert!(inst.code_path().unwrap().exists());
        let mut values = serde_json::Map::new();
        values.insert("user".into(), serde_json::json!("milo"));
        save_values("scrobbler", &values).unwrap();
        assert_eq!(load_values("scrobbler")["user"], "milo");
        uninstall("scrobbler").unwrap();
        uninstall("sunset").unwrap();
        assert!(installed().is_empty());
        // A folder with only a manifest is a local plugin (plugin makers).
        paths::write_json(&plugins_dir().join("sunset").join(MANIFEST_FILE), &theme("sunset", "0.1.0")).unwrap();
        let local = installed();
        assert_eq!(local.len(), 1);
        assert!(local[0].origin.is_local());
        // A folder named differently from the id is ignored.
        paths::write_json(&plugins_dir().join("other").join(MANIFEST_FILE), &theme("sunset", "0.1.0")).unwrap();
        assert_eq!(installed().len(), 1);
        uninstall("sunset").unwrap();
        assert!(uninstall("../etc").is_err());
        std::env::remove_var("GLYPH_DATA_DIR");
    }
}
