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
        max_app: None,
        changes: String::new(),
        kind: "theme".into(),
        theme: Some(ThemeSpec { accent: "#ff8a3d".into(), glow: Some(["#3a1206".into(), "#ff6516".into(), "#ffc04a".into()]), base: Some("#050302".into()) }),
        main: None,
        permissions: Vec::new(),
        network: Vec::new(),
        strings: BTreeMap::new(),
        hide: Vec::new(),
        settings: Vec::new(),
        menus: Vec::new(),
        look: None,
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
    Listing { repo: repo.into(), path: String::new(), rev: "HEAD".into(), checked, stars: None, updated: None, manifest: m, versions: Vec::new() }
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
fn looks_validate() {
    let base = r##"{"schema":2,"id":"neon","name":"Neon","version":"1.0.0","kind":"extension","look":LOOK}"##;
    let parse = |look: &str| serde_json::from_str::<Manifest>(&base.replace("LOOK", look)).unwrap();
    assert!(parse(r#"{"font":"serif","radius":1.5,"backgrounds":["bg/a.webp"],"css":"style.css"}"#).validate("1.0.0").is_ok());
    assert_eq!(parse(r#"{"font":"comic"}"#).validate("1.0.0"), Err(Invalid::NeedsNewerApp));
    assert!(matches!(parse(r#"{}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    assert!(matches!(parse(r#"{"radius":3}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    assert!(matches!(parse(r#"{"backgrounds":["../x.png"]}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    assert!(matches!(parse(r#"{"backgrounds":["x.gif"]}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    assert!(matches!(parse(r#"{"css":"x.js"}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    assert!(matches!(parse(r#"{"css":"a.css","backgrounds":["a.css"]}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    assert!(parse(r#"{"assets":["fonts/a.woff2","img/b.svg"],"icons":{"heart":"icons/heart.svg","heart-fill":"icons/heart.svg"}}"#).validate("1.0.0").is_ok());
    assert!(matches!(parse(r#"{"assets":["x.exe"]}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    assert!(matches!(parse(r#"{"icons":{"Heart":"h.svg"}}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    assert!(matches!(parse(r#"{"icons":{"heart":"h.png"}}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    // Own font files for text and mono, and an app icon.
    let own = parse(r#"{"font":"fonts/Literata.woff2","mono":"fonts/Code.ttf","app_icon":"icon.png"}"#);
    assert!(own.validate("1.0.0").is_ok());
    let l = own.look.as_ref().unwrap();
    assert_eq!(l.files(), ["fonts/Code.ttf", "fonts/Literata.woff2", "icon.png"]);
    assert_eq!(l.max_size("icon.png"), MAX_APP_ICON);
    assert_eq!(l.max_size("fonts/Code.ttf"), MAX_LOOK_ASSET);
    assert!(parse(r#"{"font":"f/a.woff2","mono":"f/a.woff2"}"#).validate("1.0.0").is_ok());
    assert!(matches!(parse(r#"{"font":"f/a.exe"}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    assert!(matches!(parse(r#"{"font":"../a.woff2"}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    assert!(matches!(parse(r#"{"font":"f/a.woff2","assets":["f/a.woff2"]}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    assert!(matches!(parse(r#"{"app_icon":"icon.svg"}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    assert!(matches!(parse(r#"{"app_icon":"a.png","backgrounds":["a.png"]}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    assert_eq!(parse(r#"{"mono":"pixel"}"#).validate("1.0.0"), Err(Invalid::NeedsNewerApp));
    // Home page, sidebar and tabs: order and hide; unknown names wait for a newer Glyph.
    assert!(parse(r#"{"home":{"order":["mixes","recent"],"hide":["trending","greeting"]},"sidebar":{"hide":["now_playing"]},"tabs":{"order":["library"]}}"#).validate("1.0.0").is_ok());
    assert!(parse(r#"{"home":{"hide":["from-the-future"]}}"#).validate("1.0.0").is_ok());
    assert!(matches!(parse(r#"{"home":{"hide":["Recent"]}}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    assert!(matches!(parse(r#"{"sidebar":{"order":["home","home"]}}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    // Sidebar on the right and an own startup animation.
    assert!(parse(r#"{"sidebar_position":"right","splash":{"art":[" /\\ ","(**)"],"tint":["    ","pppp"],"speed":1.5}}"#).validate("1.0.0").is_ok());
    assert!(matches!(parse(r#"{"sidebar_position":"top"}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    assert!(matches!(parse(r#"{"splash":{"art":[]}}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    assert!(matches!(parse(r#"{"splash":{"art":["   "]}}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    assert!(matches!(parse(r#"{"splash":{"art":["ä"]}}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    assert!(matches!(parse(r#"{"splash":{"art":["ab"],"tint":["pp","pp"]}}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    assert!(matches!(parse(r#"{"splash":{"art":["ab"],"speed":5}}"#).validate("1.0.0"), Err(Invalid::Bad(_))));
    // A theme may bring a look; a look alone makes a valid extension.
    let mut t = theme("sunset", "1.0.0");
    t.look = Some(LookSpec { font: "rounded".into(), ..Default::default() });
    assert!(t.validate("1.0.0").is_ok());
}

#[test]
fn app_icon_is_checked() {
    let png = |w: u32, h: u32| {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b
    };
    assert!(check_app_icon(&png(512, 512)).is_ok());
    assert!(check_app_icon(&png(512, 256)).is_err());
    assert!(check_app_icon(&png(32, 32)).is_err());
    assert!(check_app_icon(&png(2048, 2048)).is_err());
    assert!(check_app_icon(b"GIF89a").is_err());
    let look = LookSpec { app_icon: Some("icon.png".into()), ..Default::default() };
    assert!(check_look_file(&look, "icon.png", &png(256, 256)).is_ok());
    assert!(check_look_file(&look, "icon.png", &png(256, 100)).is_err());
}

#[test]
fn css_is_checked() {
    assert!(check_css("[data-part=\"sidebar\"] { opacity: .8 } /* } */ .a::after { content: \"}\" }", &[]).is_ok());
    assert!(check_css("@media (max-width: 900px) { .a { color: red } }", &[]).is_ok());
    // url() only for the look's own files.
    let assets = vec![String::from("img/a.png"), String::from("fonts/x.woff2")];
    assert!(check_css(".a { background: url(img/a.png) }", &assets).is_ok());
    assert!(check_css("@font-face { font-family: X; src: url( \"fonts/x.woff2\" ) format(\"woff2\") }", &assets).is_ok());
    assert!(check_css(".a { background: url(img/b.png) }", &assets).is_err());
    assert!(check_css(".a { background: url('https://x.example/a.png') }", &assets).is_err());
    assert!(check_css(".a { background: url(data:image/png;base64,AA) }", &assets).is_err());
    assert!(check_css(".a { background: url(img/a.png }", &assets).is_err());
    let inlined = inline_css(".a { background: url(img/a.png) } .b { background: url('img/a.png') }", |_| Some(vec![1, 2, 3])).unwrap();
    assert_eq!(inlined, ".a { background: url(\"data:image/png;base64,AQID\") } .b { background: url(\"data:image/png;base64,AQID\") }");
    assert!(inline_css(".a { background: url(img/a.png) }", |_| None).is_none());
    for bad in [
        "} * { display: none }",
        ".a { color: red",
        ".a { background: url(https://x.example/p.png) }",
        ".a { background: URL(x) }",
        "@import 'x.css';",
        ".a { content: '\\75 rl(' }",
        ".a { content: \"unclosed }",
        "/* open",
        "</style><script>",
    ] {
        assert!(check_css(bad, &[]).is_err(), "{bad}");
    }
    assert!(check_css(&"a{}".repeat(50_000), &[]).is_err());
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
    let entry = |repo: &str, id: &str| IndexEntry { repo: repo.into(), path: String::new(), rev: "abc".into(), stars: None, updated: None, manifest: theme(id, "1.0.0"), versions: Vec::new() };
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
    // A look: its files land in look/, all of them checked.
    let mut m = theme("neon", "1.0.0");
    let mut icons = BTreeMap::new();
    icons.insert(String::from("heart"), String::from("icons/heart.svg"));
    m.look = Some(LookSpec { css: Some("style.css".into()), backgrounds: vec!["bg/a.png".into()], icons, ..Default::default() });
    let l = listing(m, "e/f", false);
    let css = (String::from("style.css"), b".a { color: red }".to_vec());
    let svg = (String::from("icons/heart.svg"), b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec());
    let png = (String::from("bg/a.png"), b"\x89PNG\r\n\x1a\n....".to_vec());
    assert!(install_with_look(&l, None, &[css.clone(), svg.clone()]).is_err());
    assert!(install_with_look(&l, None, &[css.clone(), svg.clone(), (png.0.clone(), b"GIF89a".to_vec())]).is_err());
    assert!(install_with_look(&l, None, &[(css.0.clone(), b"} * { display: none }".to_vec()), svg.clone(), png.clone()]).is_err());
    let inst = install_with_look(&l, None, &[css, svg, png]).unwrap();
    assert_eq!(inst.look_css().as_deref(), Some(".a { color: red }"));
    assert_eq!(inst.look_backgrounds().len(), 1);
    assert!(inst.look_icons()["heart"].starts_with("<svg"));
    assert!(inst.look_file("../origin.json").is_none());
    uninstall("neon").unwrap();
    std::env::remove_var("GLYPH_DATA_DIR");
}

#[test]
fn max_app_pauses_on_a_newer_glyph() {
    assert_eq!(fits_app(None, Some("0.14.2"), "0.14.2"), Ok(()));
    assert_eq!(fits_app(None, Some("0.14.2"), "0.14.3"), Err(Invalid::NeedsOlderApp));
    // A whole minor line: every patch of it.
    assert_eq!(fits_app(None, Some("0.14"), "0.14.9"), Ok(()));
    assert_eq!(fits_app(None, Some("0.14.x"), "0.14.9"), Ok(()));
    assert_eq!(fits_app(None, Some("0.14.x"), "0.15.0"), Err(Invalid::NeedsOlderApp));
    // A pre-release of 0.14.3 counts as 0.14.3.
    assert_eq!(fits_app(None, Some("0.14.2"), "0.14.3-beta.1"), Err(Invalid::NeedsOlderApp));
    assert_eq!(fits_app(Some("0.12.0"), Some("0.14"), "0.11.0"), Err(Invalid::NeedsNewerApp));
    assert!(matches!(fits_app(None, Some("0.14.y"), "0.14.0"), Err(Invalid::Bad(_))));
    assert!(matches!(fits_app(None, Some("latest"), "0.14.0"), Err(Invalid::Bad(_))));
    assert!(matches!(fits_app(Some("0.15.0"), Some("0.14"), "0.14.0"), Err(Invalid::Bad(_))));
    let mut m = theme("sunset", "1.0.0");
    m.max_app = Some("0.5".into());
    assert_eq!(m.validate("0.6.1"), Err(Invalid::NeedsOlderApp));
    assert_eq!(m.validate("0.5.7"), Ok(()));
    m.changes = "x".repeat(MAX_CHANGES + 1);
    assert!(matches!(m.validate("0.5.7"), Err(Invalid::Bad(_))));
}

fn vref(version: &str, rev: char, max_app: Option<&str>) -> VersionRef {
    VersionRef { version: version.into(), rev: rev.to_string().repeat(40), min_app: None, max_app: max_app.map(Into::into), changes: String::new(), code: false, permissions: Vec::new() }
}

#[test]
fn index_keeps_older_versions_newest_first() {
    let e = IndexEntry {
        repo: "a/b".into(),
        path: String::new(),
        rev: "abc".into(),
        stars: None,
        updated: None,
        manifest: theme("sunset", "2.0.0"),
        versions: vec![vref("1.0.0", 'a', None), vref("2.0.0", 'b', None), vref("1.5.0", 'c', None), vref("1.5.0", 'd', None), VersionRef { rev: "short".into(), ..vref("1.2.0", 'e', None) }],
    };
    let l = listings_from_index(Index { plugins: vec![e] });
    let v: Vec<&str> = l[0].versions.iter().map(|v| v.version.as_str()).collect();
    assert_eq!(v, ["1.5.0", "1.0.0"]);
}

#[test]
fn an_earlier_version_fits_when_the_newest_does_not() {
    let mut newest = theme("sunset", "2.0.0");
    newest.min_app = Some("99.0.0".into());
    let mut l = listing(newest, "a/b", true);
    l.versions = vec![vref("1.5.0", 'a', Some("0.1")), vref("1.4.0", 'b', None), vref("1.0.0", 'c', None)];
    assert_eq!(l.fitting_version().unwrap().version, "1.4.0");
    let c = Catalog { checked: vec![l.clone()], ..Default::default() };
    let at = |v: &str| Installed {
        manifest: theme("sunset", v),
        origin: Origin { repo: "a/b".into(), path: String::new(), rev: "x".into(), checked: true, installed_at: 0 },
    };
    assert!(at("1.0.0").update_in(&c).is_none());
    assert_eq!(at("1.0.0").fitting_update_in(&c).unwrap().1.version, "1.4.0");
    assert!(at("1.4.0").fitting_update_in(&c).is_none());
    // A paused plugin may also go back to a fitting version.
    let mut paused = at("1.5.0");
    paused.manifest.max_app = Some("0.1".into());
    assert!(!paused.fits());
    assert_eq!(paused.fitting_update_in(&c).unwrap().1.version, "1.4.0");
    // The newest fits: nothing earlier is offered.
    l.manifest.min_app = None;
    assert!(l.fitting_version().is_none());
}

#[test]
fn a_manifest_saved_by_an_older_glyph_is_installed_again() {
    // An older Glyph dropped the look's app icon when it saved the manifest.
    let old = theme("sunset", "1.1.0");
    let inst = Installed { manifest: old.clone(), origin: Origin { repo: "a/b".into(), path: String::new(), rev: "a".repeat(40), checked: true, installed_at: 0 } };
    let mut full = old.clone();
    full.look = Some(LookSpec { app_icon: Some("icon.png".into()), ..Default::default() });
    let mut l = listing(full, "a/b", true);
    l.rev = "b".repeat(40);
    let c = Catalog { checked: vec![l.clone()], ..Default::default() };
    assert_eq!(inst.update_in(&c).unwrap().rev, "b".repeat(40));
    // Same manifest at another commit, or the same commit: nothing to do.
    let same = Catalog { checked: vec![Listing { manifest: old.clone(), ..l.clone() }], ..Default::default() };
    assert!(inst.update_in(&same).is_none());
    let here = Catalog { checked: vec![Listing { rev: "a".repeat(40), ..l }], ..Default::default() };
    assert!(inst.update_in(&here).is_none());
}
