//! "Not the right one" workflow: fetch replacement candidates, play a strict
//! 30-second preview of any candidate, save the listener's personal override +
//! re-download the chosen version immediately, and submit the vote to the
//! server's consensus store.

use crate::controller::Core;
use glyph_audio::{PlayerController, PlayerEvent, Status};
use glyph_protocol::{
    MatchAlternativesResponse, PlaybackSource, ProviderId, Track, WrongMatchReportRequest,
    WrongMatchReportResponse,
};
use serde_json::json;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

const PREVIEW_MAX_S: f64 = 30.0;

struct PreviewSlot {
    player: Option<Arc<PlayerController>>,
    seq: u64,
    active_key: Option<String>,
}

static PREVIEW: OnceLock<Mutex<PreviewSlot>> = OnceLock::new();

fn slot() -> &'static Mutex<PreviewSlot> {
    PREVIEW.get_or_init(|| {
        Mutex::new(PreviewSlot {
            player: None,
            seq: 0,
            active_key: None,
        })
    })
}

fn provider_slug(p: ProviderId) -> String {
    serde_json::to_value(p)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".into())
}

fn candidate_key(t: &Track) -> String {
    format!("{}:{}", provider_slug(t.source), t.id)
}

fn emit_preview(
    core: &Core,
    key: Option<&str>,
    status: &str,
    position: f64,
    duration: f64,
    error: Option<&str>,
) {
    core.emit(
        "wrong_song_preview",
        json!({
            "key": key,
            "status": status,
            "position": position,
            "duration": duration,
            "error": error,
        }),
    );
}

/// Fetches candidate replacement matches for `track` (server + local fallback).
pub async fn alternatives(
    core: &Arc<Core>,
    track: Track,
    query: Option<String>,
) -> Result<MatchAlternativesResponse, String> {
    let backend = core.backend_or_err()?;
    backend
        .match_alternatives(&track, query)
        .await
        .map_err(|e| e.to_string())
}

/// Stops any running 30-second preview.
pub fn preview_stop(core: &Arc<Core>) {
    let player = {
        let mut g = slot().lock().unwrap_or_else(|e| e.into_inner());
        g.seq = g.seq.wrapping_add(1);
        g.active_key = None;
        g.player.clone()
    };
    if let Some(p) = player {
        p.stop();
    }
    emit_preview(core, None, "stopped", 0.0, PREVIEW_MAX_S, None);
}

/// Plays up to 30 seconds of a candidate replacement track.
pub async fn preview_start(
    core: &Arc<Core>,
    track: Track,
    preview_url: Option<String>,
) -> Result<(), String> {
    let key = candidate_key(&track);
    let (seq, old_player) = {
        let mut g = slot().lock().unwrap_or_else(|e| e.into_inner());
        g.seq = g.seq.wrapping_add(1);
        g.active_key = Some(key.clone());
        (g.seq, g.player.clone())
    };
    if let Some(p) = old_player {
        p.stop();
    }
    // Pause main playback so preview audio never collides with the active song.
    if let Some(main) = &core.player {
        if main.state().status == Status::Playing {
            main.pause();
        }
    }
    emit_preview(core, Some(&key), "loading", 0.0, PREVIEW_MAX_S, None);

    let backend = core.backend_or_err()?;
    let source = match preview_url
        .as_deref()
        .map(str::trim)
        .filter(|u| u.starts_with("http://") || u.starts_with("https://"))
    {
        Some(url) => PlaybackSource::Preview {
            provider: track.source,
            url: url.to_string(),
        },
        None => match backend.resolve_exact(&track).await {
            Ok(Some(resolved)) => resolved.source,
            Ok(None) => {
                emit_preview(core, Some(&key), "error", 0.0, PREVIEW_MAX_S, Some("No preview available"));
                return Err("no preview available for this candidate".into());
            }
            Err(e) => {
                let msg = e.to_string();
                emit_preview(core, Some(&key), "error", 0.0, PREVIEW_MAX_S, Some(&msg));
                return Err(msg);
            }
        },
    };

    // Abort if the user already stopped or switched to another candidate while loading.
    let player = {
        let mut g = slot().lock().unwrap_or_else(|e| e.into_inner());
        if g.seq != seq {
            return Ok(());
        }
        if g.player.is_none() {
            let p = PlayerController::new().map_err(|e| e.to_string())?;
            g.player = Some(Arc::new(p));
        }
        g.player.clone().unwrap()
    };

    let vol = core
        .player
        .as_ref()
        .map(|p| p.state().volume)
        .unwrap_or_else(|| core.st().config.player.volume);
    player.set_volume(vol);
    player.load(track.clone(), source);
    player.play();

    let core_task = core.clone();
    let key_task = key.clone();
    crate::rt::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(150));
        loop {
            tick.tick().await;
            if slot().lock().unwrap_or_else(|e| e.into_inner()).seq != seq {
                return;
            }
            // Drain events first to catch decode/network errors or natural end.
            while let Ok(ev) = player.events().try_recv() {
                match ev {
                    PlayerEvent::Ended => {
                        player.stop();
                        emit_preview(&core_task, None, "stopped", 0.0, PREVIEW_MAX_S, None);
                        return;
                    }
                    PlayerEvent::Error(msg) => {
                        player.stop();
                        emit_preview(&core_task, Some(&key_task), "error", 0.0, PREVIEW_MAX_S, Some(&msg));
                        return;
                    }
                    _ => {}
                }
            }
            let st = player.state();
            let pos = st.position.as_secs_f64();
            let dur = st
                .duration
                .map(|d| d.as_secs_f64().min(PREVIEW_MAX_S))
                .unwrap_or(PREVIEW_MAX_S);
            if pos >= PREVIEW_MAX_S {
                player.stop();
                emit_preview(&core_task, None, "stopped", 0.0, PREVIEW_MAX_S, None);
                return;
            }
            let status = match st.status {
                Status::Loading => "loading",
                Status::Playing => "playing",
                Status::Paused => "paused",
                Status::Ended | Status::Idle => "stopped",
            };
            emit_preview(&core_task, Some(&key_task), status, pos.min(PREVIEW_MAX_S), dur, None);
            if status == "stopped" && pos > 0.0 {
                return;
            }
        }
    });

    Ok(())
}

/// Applies the user's chosen replacement locally, removes any old wrong download,
/// starts downloading the newly selected version immediately, reloads playback
/// if this track is currently playing, and submits the vote to the server
/// consensus store (never switching other users on a single vote).
pub async fn select(
    core: &Arc<Core>,
    original: Track,
    chosen: Track,
    wrong_provider: Option<ProviderId>,
    wrong_id: Option<String>,
) -> Result<WrongMatchReportResponse, String> {
    preview_stop(core);
    let backend = core.backend_or_err()?;

    // 1. Save personal pinned override so playback & downloads use `chosen` right away.
    backend.set_pinned_match(&original, &chosen);

    // 2. Drop any previously downloaded wrong audio file & cached stream.
    let _ = backend.remove_download(&original);
    backend.clear_stream_cache_for(&original);
    core.emit_library();

    // 3. Download the newly selected version right away.
    crate::storage::download_tracks(core, vec![original.clone()], false);

    // 4. If this track is currently playing/loaded, switch playback seamlessly.
    let is_current = core
        .st()
        .current
        .as_ref()
        .is_some_and(|c| glyph_core::dedupe::track_key(c) == glyph_core::dedupe::track_key(&original));
    if is_current {
        core.start_track(original.clone());
    }

    // 5. Submit anonymous vote to the server (stored without switching everyone
    //    until enough distinct listeners agree & the server verifies it).
    let report = backend
        .report_wrong_match(WrongMatchReportRequest {
            track: original.clone(),
            wrong_provider,
            wrong_id,
            chosen: chosen.clone(),
        })
        .await
        .ok()
        .flatten()
        .unwrap_or(WrongMatchReportResponse {
            recorded: false,
            promoted: false,
            votes: 0,
            threshold: 3,
        });

    if report.promoted {
        core.toast_sub("toast.wrong_song_promoted", &original.title, "ok");
    } else {
        core.toast_sub("toast.wrong_song_saved", &original.title, "ok");
    }

    Ok(report)
}
