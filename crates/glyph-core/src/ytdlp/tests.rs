use super::*;

const RESOLVE_FIXTURE: &str = r#"{
    "id": "dQw4w9WgXcQ", "title": "Rick Astley - Never Gonna Give You Up (Official Video)",
    "channel": "Rick Astley", "uploader": "RickAstleyVEVO", "duration": 213.0, "live_status": "not_live",
    "format_id": "140", "ext": "m4a", "acodec": "mp4a.40.2",
    "url": "https://rr3---sn-x.googlevideo.com/videoplayback?expire=1760700000&ei=abc&itag=140&mime=audio%2Fmp4",
    "formats": [{"format_id": "251", "ext": "webm", "acodec": "opus", "url": "https://rr3---sn-x.googlevideo.com/videoplayback?itag=251"}]
}"#;

const SEARCH_FIXTURE: &str = r#"{
    "_type": "playlist", "id": "believer", "title": "believer",
    "entries": [
        {"_type": "url", "ie_key": "Youtube", "id": "7wtfhZwyrcc", "url": "https://www.youtube.com/watch?v=7wtfhZwyrcc",
         "title": "Imagine Dragons - Believer (Official Music Video)", "channel": "ImagineDragonsVEVO", "duration": 217.0, "live_status": null},
        {"_type": "url", "ie_key": "Youtube", "id": "IhP3J0j9JmY", "title": "Believer", "channel": "Imagine Dragons - Topic", "duration": 204},
        {"_type": "url", "ie_key": "YoutubeTab", "id": "UCT9zcQNlyht7fRlcjmflRSA", "title": "Imagine Dragons"},
        {"_type": "url", "ie_key": "Youtube", "id": "liveliveliv", "title": "24/7 radio", "channel": "x", "live_status": "is_live"},
        {"_type": "url", "ie_key": "Youtube", "id": "7wtfhZwyrcc", "title": "duplicate", "channel": "x"},
        {"broken": true}
    ]
}"#;

#[test]
fn video_ids_are_strict() {
    assert!(valid_video_id("dQw4w9WgXcQ"));
    assert!(valid_video_id("a-b_c-d_e-f"));
    for bad in ["", "dQw4w9WgXc", "dQw4w9WgXcQQ", "--exec=cal", "dQw4w9WgX/Q", "https://x.y", "dQw4w9WgXc ", "dQw4w9WgXcä"] {
        assert!(!valid_video_id(bad), "{bad:?} must be rejected");
    }
}

#[test]
fn resolve_args_are_fixed_and_end_with_the_url() {
    let a = resolve_args("dQw4w9WgXcQ", None).unwrap();
    assert_eq!(a.last().unwrap(), "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
    assert_eq!(a[a.len() - 2], "--");
    for must in ["--ignore-config", "--no-cookies", "--no-playlist", "-J"] {
        assert!(a.iter().any(|x| x == must), "missing {must}");
    }
    assert!(!a.iter().any(|x| x.contains("cookies-from-browser") || x == "--username" || x == "--js-runtimes"));
    let f = a.iter().position(|x| x == "-f").unwrap();
    assert!(a[f + 1].starts_with("bestaudio[ext=m4a]"));
    assert!(resolve_args("--exec=rm -rf", None).is_err());
    assert!(resolve_args("https://evil.example/x", None).is_err());

    let node = JsRuntime { name: "node", path: "node".into() };
    let a = resolve_args("dQw4w9WgXcQ", Some(&node)).unwrap();
    let i = a.iter().position(|x| x == "--js-runtimes").unwrap();
    assert_eq!(a[i + 1], "node");
    let deno = JsRuntime { name: "deno", path: "deno".into() };
    assert!(!resolve_args("dQw4w9WgXcQ", Some(&deno)).unwrap().iter().any(|x| x == "--js-runtimes"));
}

#[test]
fn search_args_keep_the_query_one_argument() {
    let a = search_args("  --exec evil\n\tsong  ", 10, None).unwrap();
    assert_eq!(a.last().unwrap(), "ytsearch10:--exec evil song");
    assert_eq!(a[a.len() - 2], "--");
    assert!(a.iter().any(|x| x == "--flat-playlist") && a.iter().any(|x| x == "--dump-single-json"));
    assert!(search_args("x", 999, None).unwrap().last().unwrap().starts_with("ytsearch25:"));
    assert!(search_args(" \n ", 5, None).is_err());
    let long = "a".repeat(500);
    assert_eq!(search_args(&long, 5, None).unwrap().last().unwrap().len(), "ytsearch5:".len() + 200);
}

#[test]
fn parses_resolve_output() {
    let a = parse_resolve(RESOLVE_FIXTURE.as_bytes(), 1_760_000_000).unwrap();
    assert_eq!(a.video_id, "dQw4w9WgXcQ");
    assert!(a.url.contains("itag=140"));
    assert_eq!(a.mime.as_deref(), Some("audio/mp4"));
    assert_eq!(a.duration_ms, Some(213_000));
    assert_eq!(a.channel, "Rick Astley");
    assert_eq!(a.expires, 1_760_700_000);

    // Format only in the list, no expire param.
    let j = r#"{"id":"dQw4w9WgXcQ","title":"t","format_id":"140","formats":[{"format_id":"140","ext":"m4a","acodec":"mp4a.40.2","url":"https://g/v?itag=140"}]}"#;
    let a = parse_resolve(j.as_bytes(), 100).unwrap();
    assert_eq!(a.url, "https://g/v?itag=140");
    assert_eq!(a.expires, 100 + URL_DEFAULT_TTL_S);

    assert!(parse_resolve(b"not json", 0).is_err());
    assert!(parse_resolve(br#"{"id":"dQw4w9WgXcQ","title":"t","url":"http://plain"}"#, 0).is_err());
    assert!(parse_resolve(br#"{"id":"bad","url":"https://x"}"#, 0).is_err());
    assert!(parse_resolve(br#"{"id":"dQw4w9WgXcQ","url":"https://x","live_status":"is_live"}"#, 0).is_err());
}

#[test]
fn parses_search_output() {
    let c = parse_search(SEARCH_FIXTURE.as_bytes()).unwrap();
    let ids: Vec<&str> = c.iter().map(|c| c.track.id.as_str()).collect();
    assert_eq!(ids, ["7wtfhZwyrcc", "IhP3J0j9JmY"]);
    assert_eq!(c[0].track.artist, "Imagine Dragons");
    assert_eq!(c[0].track.title, "Believer");
    assert_eq!(c[0].track.duration_ms, Some(217_000));
    assert_eq!(c[0].track.source, glyph_protocol::ProviderId::YouTube);
    // Topic channel: clean metadata and a bonus
    assert_eq!(c[1].track.artist, "Imagine Dragons");
    assert!(c[1].bonus > c[0].bonus);
    assert!(parse_search(br#"{"entries":[]}"#).unwrap().is_empty());
    assert!(parse_search(b"ERROR: no json").is_err());
}

#[test]
fn errors_are_classified() {
    let e = classify("[youtube] x: Downloading webpage\nERROR: [youtube] dQw4w9WgXcQ: Video unavailable\n");
    assert!(matches!(e.downcast_ref::<ProviderError>(), Some(ProviderError::NotFound)));
    let e = classify("ERROR: [youtube] x: Sign in to confirm you're not a bot.");
    assert!(crate::provider::should_cool_down(&e));
    assert!(e.to_string().contains("Sign in to confirm"));
    assert_eq!(error_line(""), "yt-dlp failed");
}

#[test]
fn sums_and_assets() {
    let h = "a".repeat(64);
    let text = format!("{}  yt-dlp\n{h}  yt-dlp.exe\n{}  yt-dlp_macos\n", "b".repeat(64), "c".repeat(64));
    assert_eq!(parse_sums(&text, "yt-dlp.exe"), Some(h));
    assert_eq!(parse_sums(&text, "yt-dlp_linux"), None);
    assert_eq!(parse_sums("short  yt-dlp.exe", "yt-dlp.exe"), None);
    #[cfg(all(windows, target_arch = "x86_64"))]
    assert_eq!(release_asset(), Some("yt-dlp_win.zip"));
    #[cfg(target_os = "macos")]
    assert_eq!(release_asset(), Some("yt-dlp_macos.zip"));
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    assert_eq!(release_asset(), Some("yt-dlp_linux.zip"));
}

fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
    use std::io::Write;
    let mut w = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in entries {
        if name.ends_with('/') {
            w.add_directory(*name, opts).unwrap();
        } else {
            w.start_file(*name, opts).unwrap();
            w.write_all(data).unwrap();
        }
    }
    w.finish().unwrap();
}

#[test]
fn unpacks_onedir_release() {
    let dir = tempfile::tempdir().unwrap();
    let zip_path = dir.path().join("r.zip");
    let exe = exe_name(if cfg!(windows) { "yt-dlp" } else { "yt-dlp_linux" });
    write_zip(&zip_path, &[("_internal/", b""), ("_internal/base_library.zip", b"x"), (exe.as_str(), b"bin")]);
    let dest = dir.path().join("out");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(dest.join("stale"), b"old").unwrap();
    let got = unpack(&zip_path, &dest).unwrap();
    assert_eq!(got, dest.join(&exe));
    assert!(dest.join("_internal").join("base_library.zip").is_file());
    assert!(!dest.join("stale").exists(), "old files are removed");

    // No executable inside: refused.
    write_zip(&zip_path, &[("_internal/x", b"x")]);
    assert!(unpack(&zip_path, &dir.path().join("out2")).is_err());
    // Entries escaping the folder never land outside it.
    write_zip(&zip_path, &[("../escaped", b"x"), (exe.as_str(), b"bin")]);
    let _ = unpack(&zip_path, &dir.path().join("out3"));
    assert!(!dir.path().join("escaped").exists());
}

#[test]
fn batch_args_and_output() {
    let a = batch_args(&["dQw4w9WgXcQ", "7wtfhZwyrcc"], None).unwrap();
    assert!(a.iter().any(|x| x == "-j") && a.iter().any(|x| x == "--ignore-errors"));
    let dash = a.iter().position(|x| x == "--").unwrap();
    assert_eq!(&a[dash + 1..], ["https://www.youtube.com/watch?v=dQw4w9WgXcQ", "https://www.youtube.com/watch?v=7wtfhZwyrcc"]);
    assert!(batch_args(&["dQw4w9WgXcQ", "--exec=x"], None).is_err());
    assert!(batch_args(&[], None).is_err());

    let one_line = RESOLVE_FIXTURE.replace('\n', " ");
    let second = one_line.replace("dQw4w9WgXcQ", "7wtfhZwyrcc");
    let out = format!("{one_line}\nnot json\n\n{second}\n");
    let got = parse_batch(out.as_bytes(), 0);
    assert_eq!(got.iter().map(|a| a.video_id.as_str()).collect::<Vec<_>>(), ["dQw4w9WgXcQ", "7wtfhZwyrcc"]);
    assert!(parse_batch(b"", 0).is_empty());
}

#[test]
fn url_cache_respects_expiry() {
    let mut a = parse_resolve(RESOLVE_FIXTURE.as_bytes(), 0).unwrap();
    a.video_id = "cacheTest01".into();
    a.expires = 10_000;
    store(&a);
    assert!(cached("cacheTest01", 10_000 - URL_MARGIN_S - 1).is_some());
    assert!(cached("cacheTest01", 10_000 - URL_MARGIN_S).is_none());
    assert_eq!(url_expiry("https://x/v?a=1&expire=42&b=2"), Some(42));
    assert_eq!(url_expiry("https://x/v"), None);
}

/// Network + real binary: `GLYPH_DATA_DIR=<scratch> cargo test -p glyph-core live_ -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn live_install_resolve_and_fetch() {
    let http = reqwest::Client::builder().user_agent("glyph-test").build().unwrap();
    if locate(None).is_none() {
        println!("installed {}", install(&http).await.unwrap());
    }
    let st = status(None).await;
    println!("{st:?}");
    let yt = YtDlp::locate(None).unwrap();
    let t = std::time::Instant::now();
    if let Some(bin) = &yt.bin {
        let _ = version(bin).await;
    }
    println!("start-up: {:?}", t.elapsed());
    let hits = yt.search("Imagine Dragons - Believer", 5).await.unwrap();
    println!("search: {:?}", hits.iter().map(|c| (&c.track.id, &c.track.artist, &c.track.title)).collect::<Vec<_>>());
    assert!(!hits.is_empty());
    let a = yt.resolve(&hits[0].track.id).await.unwrap();
    println!("mime {:?} dur {:?} expires {}", a.mime, a.duration_ms, a.expires);
    let r = http.get(&a.url).header("Range", "bytes=0-65535").send().await.unwrap();
    println!("fetch: {} {:?}", r.status(), r.headers().get("content-type"));
    assert_eq!(r.status().as_u16(), 206);
    // Batch: the other hits in one run.
    forget_urls();
    let ids: Vec<String> = hits.iter().skip(1).take(3).map(|c| c.track.id.clone()).collect();
    let t = std::time::Instant::now();
    let n = yt.resolve_many(&ids).await.unwrap();
    println!("batch: {n}/{} in {:?}", ids.len(), t.elapsed());
    assert!(n >= 2);
    // Whole song into the audio cache (what playback uses).
    let t = std::time::Instant::now();
    let file = yt.audio_file(&http, &ids[0]).await.unwrap();
    let len = std::fs::metadata(&file).unwrap().len();
    println!("audio file: {} ({len} bytes) in {:?}", file.display(), t.elapsed());
    assert!(len > 500_000);
    let t = std::time::Instant::now();
    assert_eq!(yt.audio_file(&http, &ids[0]).await.unwrap(), file);
    println!("cached replay: {:?}", t.elapsed());
}

#[test]
fn audio_cache_and_headers() {
    let a = parse_resolve(RESOLVE_FIXTURE.as_bytes(), 0).unwrap();
    assert!(a.headers.is_empty());
    let j = r#"{"id":"dQw4w9WgXcQ","title":"t","url":"https://g/v","ext":"m4a","http_headers":{"User-Agent":"UA","Cookie":"x","Accept":"*/*"}}"#;
    let a = parse_resolve(j.as_bytes(), 0).unwrap();
    let mut h: Vec<&str> = a.headers.iter().map(|(k, _)| k.as_str()).collect();
    h.sort();
    assert_eq!(h, ["Accept", "User-Agent"], "cookies are never forwarded");

    assert_eq!(ext_for(Some("audio/mp4")), "m4a");
    assert_eq!(ext_for(Some("audio/mpeg")), "mp3");
    assert_eq!(mime_for_path(Path::new("x/abc.m4a")), "audio/mp4");

    let dir = tempfile::tempdir().unwrap();
    for (i, name) in ["old.m4a", "mid.m4a", "new.m4a"].iter().enumerate() {
        let p = dir.path().join(name);
        std::fs::write(&p, vec![0u8; 100]).unwrap();
        let t = std::time::SystemTime::now() - Duration::from_secs(300 - i as u64 * 100);
        std::fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
    }
    prune_audio_cache(dir.path(), 200);
    assert!(!dir.path().join("old.m4a").exists());
    assert!(dir.path().join("mid.m4a").exists() && dir.path().join("new.m4a").exists());
    assert!(cached_audio("not-an-id").is_none());
}

#[test]
fn forbidden_is_detected() {
    assert!(is_forbidden(&anyhow::Error::from(ProviderError::Http(403))));
    assert!(!is_forbidden(&anyhow::Error::from(ProviderError::Http(500))));
    assert!(!is_forbidden(&anyhow!("other")));
}

#[test]
fn locate_prefers_configured_file() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join(exe_name("my-ytdlp"));
    std::fs::write(&f, b"").unwrap();
    assert_eq!(locate(Some(f.to_str().unwrap())), Some(f));
}
