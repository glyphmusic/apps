use super::*;

#[tokio::test]
async fn configured_server_handles_known_tracks_and_errors_without_direct_fallback() {
    use std::io::{Read, Write};
    let _g = paths::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("GLYPH_DATA_DIR", dir.path().join("data"));
    std::env::set_var("GLYPH_CACHE_DIR", dir.path().join("cache"));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let track = Track {
        id: "known".into(), source: ProviderId::Audius, title: "Song".into(), artist: "Artist".into(),
        album: None, duration_ms: None, cover_url: None, isrc: None, permalink: None, external_ids: Default::default(), explicit: false,
    };
    let resolved = serde_json::json!({"matched":track,"source":{"kind":"stream","provider":"audius","url":"/v1/audio/audius/known"},"confidence":1.0}).to_string();
    let fixture = std::thread::spawn(move || {
        let mut targets = Vec::new();
        let replies = [
            (200, serde_json::json!({"server_version":"test","protocol_version":1,"providers":["audius","youtube"]}).to_string()),
            (200, resolved),
            (503, serde_json::json!({"code":"unavailable","message":"test outage"}).to_string()),
        ];
        for (status,body) in replies {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut socket = loop {
                match listener.accept() {
                    Ok((s,_)) => break s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(5)),
                    Err(e) => panic!("missing server request: {e}"),
                }
            };
            // The listener is non-blocking; accepted sockets inherit that on Windows.
            socket.set_nonblocking(false).unwrap();
            socket.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0;4096];
                let n = socket.read(&mut chunk).unwrap();
                if n == 0 { break; }
                request.extend_from_slice(&chunk[..n]);
                if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                    let len = headers.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(0);
                    if request.len() >= end + 4 + len { break; }
                }
            }
            targets.push(String::from_utf8_lossy(&request).lines().next().unwrap().to_string());
            write!(socket, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        targets
    });
    let mut config = Config::default();
    config.server_url = base.clone();
    // Server-side YouTube with yt-dlp (not the r-dlp local default).
    config.providers.youtube_mode = crate::config::YouTubeMode::Server;
    config.providers.extractor = crate::config::Extractor::YtDlp;
    let backend = Backend::new(config).await.unwrap();
    assert!(backend.active_providers().contains(&ProviderId::YouTube));
    let result = backend.resolve(&track).await.unwrap().unwrap();
    assert!(matches!(result.source, PlaybackSource::Stream { url, .. } if url == format!("{base}/v1/audio/audius/known")));
    assert!(backend.resolve(&track).await.is_err(), "server errors must not use cached direct URLs");
    assert_eq!(fixture.join().unwrap(), ["GET /v1/health HTTP/1.1", "POST /v1/resolve HTTP/1.1", "POST /v1/resolve HTTP/1.1"]);
    // Even with a configured (now offline) server, saved audio wins before
    // warm-cache lookups, resolver requests or network authentication.
    let source = dir.path().join("saved.mp3");
    std::fs::write(&source, b"saved audio").unwrap();
    let saved = backend.downloads.store_copy(&track, ProviderId::Audius, &source).unwrap();
    backend.warm(vec![track.clone()]);
    for _ in 0..2 {
        let result = backend.resolve(&track).await.unwrap().unwrap();
        assert!(matches!(result.source, PlaybackSource::Stream { url, .. } if url == saved.to_string_lossy()));
    }
    std::env::remove_var("GLYPH_DATA_DIR");
    std::env::remove_var("GLYPH_CACHE_DIR");
}

#[tokio::test]
async fn set_config_reevaluates_providers_priority_and_server() {
    let _g = paths::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("GLYPH_DATA_DIR", dir.path().join("data"));
    std::env::set_var("GLYPH_CACHE_DIR", dir.path().join("cache"));

    let mut direct = Config::default();
    direct.server_url = "off".into();
    direct.providers.youtube_mode = crate::config::YouTubeMode::Server;
    direct.providers.extractor = crate::config::Extractor::YtDlp;
    let b = Backend::new(direct.clone()).await.unwrap();
    assert_eq!(b.mode(), BackendMode::Direct);
    assert_eq!(b.active_providers().contains(&ProviderId::YouTube), ytdlp::locate(None).is_some());
    assert!(!b.active_providers().contains(&ProviderId::Jamendo));

    let mut cfg = direct;
    cfg.keys.youtube = Some("k".into());
    cfg.keys.jamendo_client_id = Some("j".into());
    cfg.providers.enabled.push(ProviderId::YouTube);
    cfg.providers.priority = vec![ProviderId::YouTube, ProviderId::Audius];
    cfg.language = "de".into();
    b.provider_cooling_down(ProviderId::Audius);
    b.set_config(cfg).await.unwrap();
    let active = b.active_providers();
    assert!(active.contains(&ProviderId::YouTube) && active.contains(&ProviderId::Jamendo));
    assert_eq!(b.config().resolved_language(), "de");
    assert_eq!(b.config().providers.priority[0], ProviderId::YouTube);

    // Unreachable configured server stays in server mode; no silent direct traffic.
    let mut cfg2 = Config::clone(&b.config());
    cfg2.server_url = "http://127.0.0.1:9".into();
    b.set_config(cfg2).await.unwrap();
    assert!(matches!(b.mode(), BackendMode::Server { .. }));

    std::env::remove_var("GLYPH_DATA_DIR");
    std::env::remove_var("GLYPH_CACHE_DIR");
}
