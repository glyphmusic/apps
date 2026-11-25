use super::*;

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
    /// Font, corners, background pictures, icons and a style sheet (pure data).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<LookSpec>,
}

/// How a plugin changes Glyph's look beyond colours. Files are paths in the
/// repo, next to the manifest.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LookSpec {
    /// Text font: one of [`FONTS`], or the path of a font file (.woff2,
    /// .woff, .ttf, .otf) in the repo that Glyph then uses everywhere;
    /// empty = Glyph's (Inter).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub font: String,
    /// Monospaced font (numbers, ASCII art): like `font`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mono: String,
    /// The app icon (window, taskbar, tray, Dock, Android Recents): a
    /// square .png in the repo, 64 to 1024 pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_icon: Option<String>,
    /// Corner roundness: 0 square, 1 Glyph's, 2 very round.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    /// Pictures (.png, .jpg, .webp) offered in Settings â€º Appearance â€º
    /// Background picture.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backgrounds: Vec<String>,
    /// Style sheet (.css) for the whole interface (see [`check_css`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub css: Option<String>,
    /// Files the style sheet may use with `url(...)`: fonts (.woff2, .woff,
    /// .ttf, .otf) and pictures (.png, .jpg, .webp, .gif, .svg).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<String>,
    /// Glyph icons the look replaces: icon name (`heart`, `play`, ...) ->
    /// .svg file. Drawn as a mask in the text colour, like Glyph's own.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub icons: BTreeMap<String, String>,
    /// Sections of the home page to put in order or hide ([`HOME_PARTS`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<LayoutSpec>,
    /// Entries of the sidebar to put in order or hide ([`SIDEBAR_PARTS`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar: Option<LayoutSpec>,
    /// Tabs at the bottom on phones to put in order or hide ([`TAB_PARTS`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tabs: Option<LayoutSpec>,
}

/// Order and hidden parts of one area (home page, sidebar, tabs). Parts not
/// in `order` follow in Glyph's order; names this Glyph does not know are
/// skipped (they come from a newer Glyph).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LayoutSpec {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hide: Vec<String>,
}

/// Home page sections a look can order and hide.
pub const HOME_PARTS: &[&str] = &["greeting", "cards", "recent", "new_releases", "mixes", "recs", "trending"];
/// Sidebar entries a look can order and hide (Settings always stays).
pub const SIDEBAR_PARTS: &[&str] = &["home", "search", "library", "now_playing", "lists"];
/// Phone tabs a look can order and hide.
pub const TAB_PARTS: &[&str] = &["home", "search", "library"];
/// Names per list of a layout.
pub const MAX_LAYOUT_PARTS: usize = 30;

impl LayoutSpec {
    fn validate(&self, area: &str) -> std::result::Result<(), String> {
        for (key, list) in [("order", &self.order), ("hide", &self.hide)] {
            if list.len() > MAX_LAYOUT_PARTS {
                return Err(format!("{area}.{key} takes up to {MAX_LAYOUT_PARTS} names"));
            }
            if list.iter().any(|n| !valid_icon_name(&n.replace('_', "-"))) {
                return Err(format!("{area}.{key} names are 1-40 of a-z, 0-9, - and _"));
            }
            let mut seen = list.clone();
            seen.sort();
            seen.dedup();
            if seen.len() != list.len() {
                return Err(format!("{area}.{key} names each only once"));
            }
        }
        Ok(())
    }
}

/// Fonts a look can pick (from the system, nothing is downloaded).
pub const FONTS: &[&str] = &["inter", "system", "rounded", "serif", "mono"];
/// Font files a look can use as `font` / `mono`.
pub const FONT_FILES: &[&str] = &["woff2", "woff", "ttf", "otf"];
/// The app icon at most this big, and its side in pixels.
pub const MAX_APP_ICON: usize = 1024 * 1024;
pub const APP_ICON_SIDES: std::ops::RangeInclusive<u32> = 64..=1024;

/// Whether a look's `font` / `mono` names a font file (not one of [`FONTS`]).
pub fn is_font_file(font: &str) -> bool {
    font.rsplit_once('.').is_some_and(|(_, e)| FONT_FILES.contains(&e.to_ascii_lowercase().as_str()))
}

/// Background pictures per plugin.
pub const MAX_LOOK_BACKGROUNDS: usize = 8;
/// One background picture at most this big.
pub const MAX_LOOK_PICTURE: usize = 8 * 1024 * 1024;
/// A look's style sheet at most this big.
pub const MAX_CSS: usize = 128 * 1024;
/// Files the style sheet may use, and the size of each.
pub const MAX_LOOK_ASSETS: usize = 40;
pub const MAX_LOOK_ASSET: usize = 2 * 1024 * 1024;
/// Icons a look may replace, and the size of each .svg.
pub const MAX_LOOK_ICONS: usize = 400;
pub const MAX_ICON: usize = 32 * 1024;
/// All files of one look together.
pub const MAX_LOOK_TOTAL: usize = 32 * 1024 * 1024;

/// File types of assets.
const ASSET_TYPES: &[&str] = &["woff2", "woff", "ttf", "otf", "png", "jpg", "jpeg", "webp", "gif", "svg"];

/// Things a look's style sheet may not use: anything that loads by itself
/// (`url()` is checked apart: only the look's assets), old script hooks, and
/// escapes that could hide them.
const CSS_BANNED: &[&str] = &[
    "@import", "@namespace", "@charset", "image-set(", "image(", "src(", "element(",
    "expression(", "javascript:", "-moz-binding", "behavior:", "\\", "<",
];

/// Where a style sheet uses `url(...)`: (start, end, the address).
fn css_urls(css: &str) -> Result<Vec<(usize, usize, String)>> {
    let lower = css.to_ascii_lowercase();
    let b = css.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(off) = lower[from..].find("url(") {
        let start = from + off;
        let mut i = start + 4;
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let addr = if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
            let q = b[i] as char;
            let Some(end) = css[i + 1..].find(q) else { bail!("the style sheet has an unclosed url(") };
            let a = css[i + 1..i + 1 + end].trim().to_string();
            i += end + 2;
            a
        } else {
            let Some(end) = css[i..].find(')') else { bail!("the style sheet has an unclosed url(") };
            let a = css[i..i + end].trim().to_string();
            i += end;
            a
        };
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if b.get(i) != Some(&b')') {
            bail!("the style sheet has an unclosed url(");
        }
        out.push((start, i + 1, addr));
        from = i + 1;
    }
    Ok(out)
}

/// Checks a look's style sheet: small, `url()` only for the look's `assets`,
/// nothing else that loads, and its braces are balanced.
pub fn check_css(css: &str, assets: &[String]) -> Result<()> {
    if css.len() > MAX_CSS {
        bail!("the style sheet is larger than {} KB", MAX_CSS / 1024);
    }
    let lower = css.to_ascii_lowercase();
    if let Some(b) = CSS_BANNED.iter().find(|b| lower.contains(*b)) {
        bail!("the style sheet may not use {b}");
    }
    for (_, _, addr) in css_urls(css)? {
        if !assets.iter().any(|a| *a == addr) {
            bail!("url({addr}) must be one of the look's assets");
        }
    }
    let bytes = css.as_bytes();
    let (mut depth, mut i) = (0i32, 0usize);
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'*') => match css[i + 2..].find("*/") {
                Some(end) => i += end + 4,
                None => bail!("the style sheet has an unclosed comment"),
            },
            q @ (b'"' | b'\'') => match css[i + 1..].find(q as char) {
                Some(end) if !css[i + 1..i + 1 + end].contains('\n') => i += end + 2,
                _ => bail!("the style sheet has an unclosed string"),
            },
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                if depth < 0 {
                    bail!("the style sheet has a }} without a {{");
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    if depth != 0 {
        bail!("the style sheet has a {{ without a }}");
    }
    Ok(())
}

/// Media type of a look file, by its extension.
pub(super) fn mime_of(name: &str) -> Option<&'static str> {
    Some(match name.rsplit_once('.')?.1.to_ascii_lowercase().as_str() {
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        _ => return None,
    })
}

/// The style sheet with every `url(file)` replaced by the file as a `data:`
/// address (`read` gives a file's checked bytes). `None` when a file is missing.
pub fn inline_css(css: &str, read: impl Fn(&str) -> Option<Vec<u8>>) -> Option<String> {
    use base64::Engine;
    let mut done: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut out = String::with_capacity(css.len());
    let mut last = 0;
    for (start, end, addr) in css_urls(css).ok()? {
        if !done.contains_key(&addr) {
            let data = format!("data:{};base64,{}", mime_of(&addr)?, base64::engine::general_purpose::STANDARD.encode(read(&addr)?));
            done.insert(addr.clone(), data);
        }
        out.push_str(&css[last..start]);
        out.push_str("url(\"");
        out.push_str(&done[&addr]);
        out.push_str("\")");
        last = end;
    }
    out.push_str(&css[last..]);
    Some(out)
}

/// Icon names: Glyph's (`heart`, `skip-forward`, `heart-fill`).
pub(super) fn valid_icon_name(n: &str) -> bool {
    (1..=40).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl LookSpec {
    fn is_empty(&self) -> bool {
        self.font.is_empty() && self.mono.is_empty() && self.app_icon.is_none() && self.home.is_none() && self.sidebar.is_none() && self.tabs.is_none() && self.radius.is_none() && self.backgrounds.is_empty() && self.css.is_none() && self.assets.is_empty() && self.icons.is_empty()
    }

    /// Every file the look needs from the repo, each once.
    pub fn files(&self) -> Vec<&String> {
        let mut v: Vec<&String> = self.css.iter().chain(&self.backgrounds).chain(&self.assets).chain(self.icons.values()).chain(self.font_files()).chain(&self.app_icon).collect();
        v.sort();
        v.dedup();
        v
    }

    /// The font files `font` and `mono` name.
    pub fn font_files(&self) -> impl Iterator<Item = &String> {
        [&self.font, &self.mono].into_iter().filter(|f| is_font_file(f))
    }

    /// Largest size file `name` may have, by what it is used for.
    pub(super) fn max_size(&self, name: &str) -> usize {
        let mut max = 0;
        if self.css.as_deref() == Some(name) {
            max = max.max(MAX_CSS);
        }
        if self.backgrounds.iter().any(|b| b == name) {
            max = max.max(MAX_LOOK_PICTURE);
        }
        if self.assets.iter().any(|b| b == name) {
            max = max.max(MAX_LOOK_ASSET);
        }
        if self.icons.values().any(|b| b == name) {
            max = max.max(MAX_ICON);
        }
        if self.font_files().any(|b| b == name) {
            max = max.max(MAX_LOOK_ASSET);
        }
        if self.app_icon.as_deref() == Some(name) {
            max = max.max(MAX_APP_ICON);
        }
        max
    }

    fn validate(&self) -> Result<(), Invalid> {
        let bad = |s: &str| Err(Invalid::Bad(format!("look: {s}")));
        if self.is_empty() {
            return bad("needs a font, mono, app_icon, radius, backgrounds, css, assets, icons, home, sidebar or tabs");
        }
        for (area, l) in [("home", &self.home), ("sidebar", &self.sidebar), ("tabs", &self.tabs)] {
            if let Some(l) = l {
                if let Err(e) = l.validate(area) {
                    return bad(&e);
                }
            }
        }
        for (key, f) in [("font", &self.font), ("mono", &self.mono)] {
            if f.is_empty() || FONTS.contains(&f.as_str()) {
                continue;
            }
            if f.contains('.') || f.contains('/') {
                if !is_font_file(f) || !valid_path(f) {
                    return bad(&format!("{key} must be one of Glyph's fonts or the path of a .woff2, .woff, .ttf or .otf file in the repo"));
                }
            } else {
                // A font this build does not know comes from a newer Glyph.
                return Err(Invalid::NeedsNewerApp);
            }
        }
        if let Some(icon) = &self.app_icon {
            if !valid_path(icon) || icon.is_empty() || !icon.to_ascii_lowercase().ends_with(".png") {
                return bad("app_icon must be the path of a .png file in the repo");
            }
        }
        if self.radius.is_some_and(|r| !(r.is_finite() && (0.0..=2.0).contains(&r))) {
            return bad("radius must be between 0 and 2");
        }
        let ext = |f: &str| f.rsplit_once('.').map(|(_, e)| e.to_string()).unwrap_or_default();
        if self.backgrounds.len() > MAX_LOOK_BACKGROUNDS {
            return bad("at most 8 backgrounds");
        }
        for b in &self.backgrounds {
            if !valid_path(b) || b.is_empty() || !matches!(ext(b).as_str(), "png" | "jpg" | "jpeg" | "webp") {
                return bad("backgrounds are paths of .png, .jpg or .webp files in the repo");
            }
        }
        if let Some(css) = &self.css {
            if !valid_path(css) || !css.ends_with(".css") {
                return bad("css must be the path of a .css file in the repo");
            }
        }
        if self.assets.len() > MAX_LOOK_ASSETS {
            return bad("at most 40 assets");
        }
        for a in &self.assets {
            if !valid_path(a) || a.is_empty() || !ASSET_TYPES.contains(&ext(a).as_str()) {
                return bad("assets are paths of font (.woff2, .woff, .ttf, .otf) or picture (.png, .jpg, .webp, .gif, .svg) files in the repo");
            }
        }
        if self.icons.len() > MAX_LOOK_ICONS {
            return bad("at most 400 icons");
        }
        for (name, file) in &self.icons {
            if !valid_icon_name(name) {
                return bad("icon names are 1-40 of a-z, 0-9 and -");
            }
            if !valid_path(file) || file.is_empty() || ext(file) != "svg" {
                return bad("icons are paths of .svg files in the repo");
            }
        }
        // Each file has one job among css, backgrounds, assets, fonts and the
        // app icon (icons may share files; font and mono may be the same file).
        let mut fonts: Vec<&String> = self.font_files().collect();
        fonts.dedup();
        let mut files: Vec<&String> = self.css.iter().chain(&self.backgrounds).chain(&self.assets).chain(fonts).chain(&self.app_icon).collect();
        let n = files.len();
        files.sort();
        files.dedup();
        if files.len() != n {
            return bad("each file only once");
        }
        Ok(())
    }
}

fn schema_one() -> u32 {
    1
}

/// Permissions a plugin's code can ask for.
pub const PERMISSIONS: &[&str] = &["playback", "library", "search", "settings", "network", "audio", "lyrics_provider"];

/// Glyph's settings pages plugins can add sections to (`page.subtab`), plus
/// `"own"`: the plugin's own page under Settings â€º Plugins.
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
pub(super) fn valid_lang(code: &str) -> bool {
    code.len() == 2 && code.bytes().all(|b| b.is_ascii_lowercase())
}

/// Ids of settings sections, items and menu entries: 1-40 of `a-z0-9_-`.
pub(super) fn valid_key(id: &str) -> bool {
    (1..=40).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// Glyph text keys (`sp.crossfade`).
pub(super) fn valid_text_key(key: &str) -> bool {
    (1..=100).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

pub(super) fn protected(key: &str) -> bool {
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
pub(super) fn valid_path(path: &str) -> bool {
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
        if let Some(l) = &self.look {
            l.validate()?;
        }
        match self.kind.as_str() {
            "theme" => {
                if self.theme.is_none() {
                    return bad("a theme plugin needs a \"theme\" object");
                }
                if self.main.is_some() || !self.strings.is_empty() || !self.hide.is_empty() || !self.settings.is_empty() || !self.menus.is_empty() {
                    return bad("a theme has colours and a look only; use \"kind\": \"extension\" for more");
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
        if self.theme.is_none() && self.look.is_none() && self.main.is_none() && self.strings.is_empty() && self.hide.is_empty() {
            return bad("an extension needs code (\"main\"), \"strings\", \"hide\", a \"theme\" or a \"look\"".into());
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
