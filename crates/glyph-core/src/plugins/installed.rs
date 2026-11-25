use super::*;

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
        let l = self.listing_in(catalog)?;
        (is_newer(&l.manifest.version, &self.manifest.version) && l.installable().is_ok()).then_some(l)
    }

    /// When the newest version does not fit this Glyph: the newest earlier
    /// one that does and is newer than this (or, for a plugin this Glyph
    /// paused, any that fits).
    pub fn fitting_update_in<'a>(&self, catalog: &'a Catalog) -> Option<(&'a Listing, &'a VersionRef)> {
        let l = self.listing_in(catalog)?;
        let v = l.fitting_version()?;
        let paused = !self.fits();
        (paused || is_newer(&v.version, &self.manifest.version)).then_some((l, v))
    }

    fn listing_in<'a>(&self, catalog: &'a Catalog) -> Option<&'a Listing> {
        catalog.find(&self.manifest.id).filter(|l| l.repo.eq_ignore_ascii_case(&self.origin.repo))
    }

    /// Fits this Glyph's version (see [`installed_unfit`] for those that do not).
    pub fn fits(&self) -> bool {
        self.manifest.validate(app_version()).is_ok()
    }
}

/// Installed plugins that are still valid for this Glyph, sorted by name.
pub fn installed() -> Vec<Installed> {
    scan().into_iter().filter_map(|(p, ok)| ok.is_ok().then_some(p)).collect()
}

/// Installed plugins this Glyph's version does not fit (`min_app` or
/// `max_app`), with why: kept but paused until an update (of Glyph or of
/// the plugin) fits.
pub fn installed_unfit() -> Vec<(Installed, Invalid)> {
    scan()
        .into_iter()
        .filter_map(|(p, ok)| match ok {
            Err(e @ (Invalid::NeedsNewerApp | Invalid::NeedsOlderApp)) => Some((p, e)),
            _ => None,
        })
        .collect()
}

fn scan() -> Vec<(Installed, Result<(), Invalid>)> {
    let Ok(dir) = std::fs::read_dir(plugins_dir()) else { return Vec::new() };
    let mut out: Vec<(Installed, Result<(), Invalid>)> = dir
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let p = e.path();
            let manifest: Manifest = paths::read_json(&p.join(MANIFEST_FILE))?;
            // No origin.json: copied in by hand to try it out (local).
            let origin: Origin = paths::read_json(&p.join("origin.json")).unwrap_or_else(Origin::local);
            // The folder name is the id; anything else was tampered with.
            if e.file_name().to_str() != Some(manifest.id.as_str()) {
                return None;
            }
            let ok = manifest.validate(app_version());
            Some((Installed { manifest, origin }, ok))
        })
        .collect();
    out.sort_by_key(|a| a.0.manifest.name.to_lowercase());
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

    /// A file of the plugin's look on disk: in `look/` for installed
    /// plugins, next to the manifest for local ones.
    pub fn look_file(&self, name: &str) -> Option<PathBuf> {
        let look = self.manifest.look.as_ref()?;
        if !look.files().iter().any(|f| *f == name) || !valid_path(name) {
            return None;
        }
        let dir = self.dir();
        Some(if self.origin.is_local() { dir.join(name) } else { dir.join(LOOK_DIR).join(name) })
    }

    /// A look file's bytes when they pass [`check_look_file`].
    fn look_bytes(&self, name: &str) -> Option<Vec<u8>> {
        let look = self.manifest.look.as_ref()?;
        let bytes = std::fs::read(self.look_file(name)?).ok()?;
        match check_look_file(look, name, &bytes) {
            Ok(()) => Some(bytes),
            Err(e) => {
                tracing::warn!(plugin = %self.manifest.id, "look: {e:#}");
                None
            }
        }
    }

    /// The look's style sheet, checked, with its assets inlined as `data:`
    /// addresses (so the UI loads nothing).
    pub fn look_css(&self) -> Option<String> {
        let name = self.manifest.look.as_ref()?.css.as_ref()?;
        let css = String::from_utf8(self.look_bytes(name)?).ok()?;
        let out = inline_css(&css, |a| self.look_bytes(a));
        if out.is_none() {
            tracing::warn!(plugin = %self.manifest.id, "look: a file the style sheet uses is missing");
        }
        out
    }

    /// The icons the look replaces: name -> SVG text.
    pub fn look_icons(&self) -> BTreeMap<String, String> {
        let Some(look) = &self.manifest.look else { return BTreeMap::new() };
        look.icons
            .iter()
            .filter_map(|(name, file)| {
                let svg = String::from_utf8(self.look_bytes(file)?).ok()?;
                (svg.len() <= MAX_ICON).then(|| (name.clone(), svg))
            })
            .collect()
    }

    /// A font file of the look (`font` or `mono`) as a `data:` address, so
    /// the UI loads nothing; `None` for Glyph's own fonts.
    pub fn look_font_data(&self, font: &str) -> Option<String> {
        use base64::Engine;
        if !is_font_file(font) {
            return None;
        }
        let bytes = self.look_bytes(font)?;
        Some(format!("data:{};base64,{}", mime_of(font)?, base64::engine::general_purpose::STANDARD.encode(bytes)))
    }

    /// The look's app icon (.png bytes), checked.
    pub fn look_app_icon(&self) -> Option<Vec<u8>> {
        self.look_bytes(self.manifest.look.as_ref()?.app_icon.as_ref()?)
    }

    /// The look's background pictures that are on disk, in manifest order
    /// (index, file).
    pub fn look_backgrounds(&self) -> Vec<(usize, PathBuf)> {
        let Some(look) = &self.manifest.look else { return Vec::new() };
        look.backgrounds.iter().enumerate().filter_map(|(i, b)| self.look_file(b).filter(|p| p.is_file()).map(|p| (i, p))).collect()
    }
}

/// Folder of an installed plugin's look files.
pub const LOOK_DIR: &str = "look";

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

/// Checks one file of `look`: the style sheet with [`check_css`], fonts and
/// pictures by size and type.
pub fn check_look_file(look: &LookSpec, name: &str, bytes: &[u8]) -> Result<()> {
    let max = look.max_size(name);
    if bytes.len() > max {
        bail!("{name}: at most {} KB", max / 1024);
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let ok = match ext.as_str() {
        "css" => {
            let css = std::str::from_utf8(bytes).map_err(|_| anyhow!("{name}: the style sheet is not UTF-8 text"))?;
            return check_css(css, &look.assets).map_err(|e| anyhow!("{name}: {e}"));
        }
        "png" if look.app_icon.as_deref() == Some(name) => return check_app_icon(bytes).map_err(|e| anyhow!("{name}: {e}")),
        "png" | "jpg" | "jpeg" | "webp" => crate::backgrounds::image_ext(bytes).is_some(),
        "gif" => bytes.starts_with(b"GIF8"),
        "svg" => std::str::from_utf8(bytes).is_ok_and(|s| s.contains("<svg")),
        "woff2" => bytes.starts_with(b"wOF2"),
        "woff" => bytes.starts_with(b"wOFF"),
        "ttf" => bytes.starts_with(&[0, 1, 0, 0]) || bytes.starts_with(b"true"),
        "otf" => bytes.starts_with(b"OTTO"),
        _ => false,
    };
    if !ok {
        bail!("{name}: not a {ext} file");
    }
    Ok(())
}

/// Checks an app icon: a PNG, square, [`APP_ICON_SIDES`] pixels.
pub fn check_app_icon(bytes: &[u8]) -> Result<()> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.len() < 24 || &bytes[12..16] != b"IHDR" {
        bail!("the app icon is not a PNG file");
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().expect("4 bytes"));
    let h = u32::from_be_bytes(bytes[20..24].try_into().expect("4 bytes"));
    if w != h || !APP_ICON_SIDES.contains(&w) {
        bail!("the app icon must be square, {} to {} pixels (it is {w} x {h})", APP_ICON_SIDES.start(), APP_ICON_SIDES.end());
    }
    Ok(())
}

/// Downloads the files of `listing`'s look (style sheet, pictures, fonts,
/// icons, at the listed commit), checked; empty without a look.
pub async fn fetch_look(http: &reqwest::Client, listing: &Listing) -> Result<Vec<(String, Vec<u8>)>> {
    let Some(look) = &listing.manifest.look else { return Ok(Vec::new()) };
    if !valid_repo(&listing.repo) || !valid_path(&listing.path) || !listing.rev.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_./".contains(&b)) {
        bail!("invalid plugin location");
    }
    let mut out = Vec::new();
    let mut total = 0usize;
    for name in look.files() {
        if !valid_path(name) {
            bail!("invalid look file");
        }
        let file = if listing.path.is_empty() { name.clone() } else { format!("{}/{name}", listing.path) };
        let url = format!("https://raw.githubusercontent.com/{}/{}/{file}", listing.repo, listing.rev);
        let r = http.get(url).send().await?.error_for_status()?;
        let max = look.max_size(name);
        if r.content_length().is_some_and(|n| n as usize > max) {
            bail!("{name} is too large");
        }
        let mut body = r.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = body.next().await {
            bytes.extend_from_slice(&chunk?);
            if bytes.len() > max {
                bail!("{name} is too large");
            }
        }
        total += bytes.len();
        if total > MAX_LOOK_TOTAL {
            bail!("the look's files are larger than {} MB together", MAX_LOOK_TOTAL / (1024 * 1024));
        }
        check_look_file(look, name, &bytes)?;
        out.push((name.clone(), bytes));
    }
    Ok(out)
}

/// Installs (or updates) `listing` with its `code` (from [`fetch_code`]).
/// Another repo's plugin with the same id is refused rather than replaced.
pub fn install(listing: &Listing, code: Option<&[u8]>) -> Result<Installed> {
    install_with_look(listing, code, &[])
}

/// [`install`] with the files of the plugin's look (from [`fetch_look`]).
pub fn install_with_look(listing: &Listing, code: Option<&[u8]>, look: &[(String, Vec<u8>)]) -> Result<Installed> {
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
    // Every file the look names, checked, and nothing else.
    let spec = listing.manifest.look.as_ref();
    let wanted: Vec<&String> = spec.map(|l| l.files()).unwrap_or_default();
    if look.iter().map(|(_, b)| b.len()).sum::<usize>() > MAX_LOOK_TOTAL {
        bail!("{id}: the look's files are too large together");
    }
    for name in &wanted {
        let Some((_, bytes)) = look.iter().find(|(n, _)| n == *name) else { bail!("{id}: {name} of the look is missing") };
        check_look_file(spec.expect("files come from the look"), name, bytes)?;
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
    // The look's files replace the old ones (written beside, then swapped).
    let look_dir = dir.join(LOOK_DIR);
    let fresh = dir.join("look.part");
    if fresh.exists() {
        std::fs::remove_dir_all(&fresh)?;
    }
    if !wanted.is_empty() {
        for name in &wanted {
            let (_, bytes) = look.iter().find(|(n, _)| n == *name).expect("checked above");
            let to = fresh.join(name);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(to, bytes)?;
        }
    }
    if look_dir.exists() {
        std::fs::remove_dir_all(&look_dir)?;
    }
    if !wanted.is_empty() {
        std::fs::rename(&fresh, &look_dir)?;
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
