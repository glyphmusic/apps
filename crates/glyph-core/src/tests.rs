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

#[test]
fn offline_audio_excludes_partial_empty_and_metadata_cache_entries() {
    let dir = tempfile::tempdir().unwrap();
    let streams = dir.path().join("streams");
    let youtube = dir.path().join("youtube");
    std::fs::create_dir_all(&streams).unwrap();
    std::fs::create_dir_all(&youtube).unwrap();
    let song = dedupe::test_track(ProviderId::Archive, "saved", "Saved", "Artist", Some(200));
    let partial = dedupe::test_track(ProviderId::Archive, "partial", "Partial", "Artist", Some(200));
    let empty = dedupe::test_track(ProviderId::Archive, "empty", "Empty", "Artist", Some(200));
    std::fs::write(streams.join(format!("{}.mp3", stream_cache_name(&song))), b"audio").unwrap();
    std::fs::write(streams.join(format!("{}.part", stream_cache_name(&partial))), b"partial").unwrap();
    std::fs::write(streams.join(format!("{}.mp3", stream_cache_name(&empty))), b"").unwrap();
    let video = dedupe::test_track(ProviderId::YouTube, "abcdefghijk", "Video", "Artist", Some(200));
    std::fs::write(youtube.join("abcdefghijk.webm"), b"audio").unwrap();
    let cached = OfflineAudio::scan_dirs(streams, youtube);
    assert!(cached.contains(&song));
    assert!(cached.contains(&video));
    assert!(!cached.contains(&partial));
    assert!(!cached.contains(&empty));
    let mut other_provider = video;
    other_provider.source = ProviderId::Archive;
    assert!(!cached.contains(&other_provider));
}

#[test]
fn disabled_deezer_drops_deezer_only_tracks_and_promotes_youtube_matches() {
    let mut merged = dedupe::test_track(ProviderId::Deezer, "dz-1", "Merged", "Artist", Some(180));
    merged.external_ids.insert(ProviderId::YouTube, "yt-1".into());
    merged.external_ids.insert(ProviderId::Deezer, "dz-1".into());
    let dz_only = dedupe::test_track(ProviderId::Deezer, "dz-2", "DZ Only", "Artist", Some(180));
    let enabled = [ProviderId::YouTube];

    let Some(SearchEvent::Results { provider, tracks }) = filter_search_event(
        SearchEvent::Results { provider: ProviderId::Deezer, tracks: vec![merged, dz_only] },
        &enabled,
    ) else {
        panic!("expected filtered results");
    };
    assert_eq!(provider, ProviderId::YouTube);
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].source, ProviderId::YouTube);
    assert_eq!(tracks[0].id, "yt-1");
    assert_eq!(tracks[0].permalink.as_deref(), Some("https://www.youtube.com/watch?v=yt-1"));
    assert!(!tracks[0].external_ids.contains_key(&ProviderId::Deezer));
    assert!(filter_search_event(
        SearchEvent::ProviderError { provider: ProviderId::Deezer, message: "err".into() },
        &enabled,
    ).is_none());
}

#[tokio::test]
async fn refresh_server_recovers_from_offline_after_server_update() {
    use std::io::{Read, Write};
    let _g = paths::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("GLYPH_DATA_DIR", dir.path().join("data"));
    std::env::set_var("GLYPH_CACHE_DIR", dir.path().join("cache"));

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());

    let fixture = std::thread::spawn(move || {
        // Initial Backend::new retries /v1/health 3 times while the server restarts,
        // then subsequent refresh_server() probes succeed with the updated version.
        let replies = [
            (503, serde_json::json!({"code":"unavailable","message":"restarting"}).to_string()),
            (503, serde_json::json!({"code":"unavailable","message":"restarting"}).to_string()),
            (503, serde_json::json!({"code":"unavailable","message":"restarting"}).to_string()),
            (200, serde_json::json!({"server_version":"0.2.0","protocol_version":1,"providers":["audius","youtube"]}).to_string()),
            (200, serde_json::json!({"server_version":"0.2.0","protocol_version":1,"providers":["audius","youtube"]}).to_string()),
        ];
        for (status, body) in replies {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut socket = loop {
                match listener.accept() {
                    Ok((s, _)) => break s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(e) => panic!("missing server request: {e}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0; 4096];
                let n = socket.read(&mut chunk).unwrap();
                if n == 0 { break; }
                request.extend_from_slice(&chunk[..n]);
                if request.windows(4).any(|v| v == b"\r\n\r\n") { break; }
            }
            write!(
                socket,
                "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ).unwrap();
        }
    });

    let mut config = Config::default();
    config.server_url = base;
    let backend = Backend::new(config).await.unwrap();
    assert!(matches!(backend.mode(), BackendMode::Server { ref server_version, .. } if server_version == "offline"));
    assert!(backend.active_providers().is_empty());

    assert!(backend.refresh_server().await.unwrap());
    assert!(matches!(backend.mode(), BackendMode::Server { ref server_version, .. } if server_version == "0.2.0"));
    assert!(backend.active_providers().contains(&ProviderId::YouTube));

    fixture.join().unwrap();
    std::env::remove_var("GLYPH_DATA_DIR");
    std::env::remove_var("GLYPH_CACHE_DIR");
}

#[tokio::test]
async fn server_download_skips_resolve_roundtrip_when_youtube_id_is_known_or_cached() {
    use std::io::{Read, Write};
    let _g = paths::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("GLYPH_DATA_DIR", dir.path().join("data"));
    std::env::set_var("GLYPH_CACHE_DIR", dir.path().join("cache"));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let fixture = std::thread::spawn(move || {
        let mut targets = Vec::new();
        let health = serde_json::json!({"server_version":"test","protocol_version":1,"providers":["audius","youtube"]}).to_string();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut socket = loop {
            match listener.accept() {
                Ok((s, _)) => break s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(5)),
                Err(e) => panic!("missing health request: {e}"),
            }
        };
        socket.set_nonblocking(false).unwrap();
        let mut request = [0u8; 4096];
        let n = socket.read(&mut request).unwrap();
        targets.push(String::from_utf8_lossy(&request[..n]).lines().next().unwrap().to_string());
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{health}", health.len()).unwrap();
        targets
    });
    let mut config = Config::default();
    config.server_url = base.clone();
    config.providers.youtube_mode = crate::config::YouTubeMode::Server;
    config.providers.youtube_only = true;
    config.providers.extractor = crate::config::Extractor::YtDlp;
    let backend = Backend::new(config).await.unwrap();
    let inner = backend.cur();
    let mut deezer = Track {
        id: "3845609381".into(), source: ProviderId::Deezer, title: "s".into(), artist: "yungalligator".into(),
        album: None, duration_ms: Some(170_000), cover_url: None, isrc: None, permalink: None, external_ids: Default::default(), explicit: false,
    };
    let yt = Track {
        id: "vEa2biPJuRg".into(), source: ProviderId::YouTube, title: "s".into(), artist: "yungalligator".into(),
        album: None, duration_ms: Some(170_000), cover_url: None, isrc: None, permalink: None, external_ids: Default::default(), explicit: false,
    };
    inner.resolver.remember(&deezer, &yt, 0.9);
    let job = downloads::Job { progress: Arc::default(), quality: crate::config::AudioQuality::High };
    let (provider, url) = downloads::run_job(job, inner.download_url(&deezer)).await.unwrap().unwrap();
    assert_eq!(provider, ProviderId::YouTube);
    assert_eq!(url, format!("{base}/v1/download/youtube/vEa2biPJuRg?quality=high"));
    // Also works when the track carries the YouTube id directly.
    deezer.external_ids.insert(ProviderId::YouTube, "abcdefghijk".into());
    let job2 = downloads::Job { progress: Arc::default(), quality: crate::config::AudioQuality::Standard };
    let (provider2, url2) = downloads::run_job(job2, inner.download_url(&deezer)).await.unwrap().unwrap();
    assert_eq!(provider2, ProviderId::YouTube);
    assert_eq!(url2, format!("{base}/v1/download/youtube/abcdefghijk?quality=standard"));
    // Only `/v1/health` was requested — zero `POST /v1/download` roundtrips!
    assert_eq!(fixture.join().unwrap(), ["GET /v1/health HTTP/1.1"]);
    std::env::remove_var("GLYPH_DATA_DIR");
    std::env::remove_var("GLYPH_CACHE_DIR");
}

#[tokio::test]
#[ignore]
async fn live_benchmark_download_preparing_and_total_speed() {
    let _g = paths::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("GLYPH_DATA_DIR", dir.path().join("data"));
    std::env::set_var("GLYPH_CACHE_DIR", dir.path().join("cache"));

    let mut config = Config::default();
    config.server_url = String::new(); // official https://api.glyph.my
    config.providers.youtube_mode = crate::config::YouTubeMode::Server;
    config.providers.youtube_only = true;
    config.audio.download_quality = crate::config::AudioQuality::High;
    let backend = Arc::new(Backend::new(config).await.unwrap());

    // Brand-new cold Deezer track (never played, not in local resolver_cache).
    let cold = Track {
        id: "108481364".into(),
        source: ProviderId::Deezer,
        title: "Mein Nachbar".into(),
        artist: "Kellerkommando".into(),
        album: Some("Belzebub".into()),
        duration_ms: Some(197_000),
        cover_url: None,
        isrc: Some("DEA621500610".into()),
        permalink: None,
        external_ids: Default::default(),
        explicit: false,
    };

    // Measure exact "Preparing download" duration (until fraction() becomes Some)
    // and total download time for our NEW Backend::download() path!
    let t0 = std::time::Instant::now();
    let b_poll = backend.clone();
    let t_poll = cold.clone();
    let monitor = tokio::spawn(async move {
        let mut first_progress = None;
        loop {
            if let Some(Some(frac)) = b_poll.download_progress(&t_poll) {
                first_progress = Some((t0.elapsed(), frac));
                break;
            }
            if t0.elapsed() > std::time::Duration::from_secs(30) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        first_progress
    });
    let saved = backend.download(&cold).await.unwrap().expect("downloaded file");
    let total_cold = t0.elapsed();
    let preparing_cold = monitor.await.unwrap();
    let bytes = std::fs::metadata(&saved).unwrap().len();
    eprintln!(
        "LIVE NEW (cold track, no cache): preparing={:?}, total={:.2}s, size={} B ({:.2} MB/s)",
        preparing_cold.map(|(d, f)| format!("{:.0} ms (first frac {:.1}%)", d.as_secs_f64() * 1000.0, f * 100.0)),
        total_cold.as_secs_f64(),
        bytes,
        (bytes as f64 / (1024.0 * 1024.0)) / total_cold.as_secs_f64()
    );
    assert!(bytes > 100_000);
    assert!(preparing_cold.is_some(), "must leave Preparing download before completion");
    assert!(total_cold.as_secs_f64() < 10.0, "cold ranged download should take a few seconds, not ~109s");

    // Now remove download, keep resolver_cache (simulates hovered/previously-resolved song):
    assert!(backend.remove_download(&cold));
    let t1 = std::time::Instant::now();
    let b_poll2 = backend.clone();
    let t_poll2 = cold.clone();
    let monitor2 = tokio::spawn(async move {
        loop {
            if let Some(Some(frac)) = b_poll2.download_progress(&t_poll2) {
                return Some((t1.elapsed(), frac));
            }
            if t1.elapsed() > std::time::Duration::from_secs(30) {
                return None;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    });
    let saved2 = backend.download(&cold).await.unwrap().expect("downloaded file 2");
    let total_cached_id = t1.elapsed();
    let preparing_cached_id = monitor2.await.unwrap();
    let bytes2 = std::fs::metadata(&saved2).unwrap().len();
    eprintln!(
        "LIVE NEW (cached YouTube ID): preparing={:?}, total={:.2}s, size={} B ({:.2} MB/s)",
        preparing_cached_id.map(|(d, f)| format!("{:.0} ms (first frac {:.1}%)", d.as_secs_f64() * 1000.0, f * 100.0)),
        total_cached_id.as_secs_f64(),
        bytes2,
        (bytes2 as f64 / (1024.0 * 1024.0)) / total_cached_id.as_secs_f64()
    );
    assert_eq!(bytes, bytes2);

    std::env::remove_var("GLYPH_DATA_DIR");
    std::env::remove_var("GLYPH_CACHE_DIR");
}
