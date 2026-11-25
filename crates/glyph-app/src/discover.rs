//! Radio, recommendations, lyrics extras, statistics, sharing, sleep timer
//! (src/app/playback.rs, menus.rs, lyrics.rs, lists.rs, library.rs,
//! player_state.rs, sleep.rs, ui/stats.rs).
//!
//! Background parts need hooks from controller.rs; they are the `pub fn`s
//! `on_tick`, `on_track_started`, `on_song_ended`, `on_queue_end`, `on_skip`,
//! `on_stats_toggle`, `sleep_blocks_preload`, `set_mpd_status`,
//! `set_device_status`.

use crate::controller::Core;
use crate::dto;
use futures::StreamExt;
use glyph_audio::Status;
use glyph_core::dedupe::{by_artist, normalize_artist, primary_artist, same_song, track_key};
use glyph_core::stats::{self, Range, When};
use glyph_core::{ListKey, Lyrics};
use glyph_protocol::{ProviderId, SearchEvent, Track};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Res<T = ()> = Result<T, String>;
type C<'a> = &'a Arc<Core>;

/// The sleep timer turns the music down over its last stretch.
const FADE: Duration = Duration::from_secs(30);
/// Radio answer asked again after this long (settings page).
const RADIO_PROBE_TTL: Duration = Duration::from_secs(300);

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Clone, Copy, PartialEq)]
enum Sleep {
    At(Instant),
    EndOfSong,
}

struct Inner {
    radio_on: bool,
    /// Glyph Drift Radio (beta) station, while it runs.
    drift: Option<glyph_core::drift::Drift>,
    /// A Drift refill is on its way.
    drift_loading: bool,
    /// Radio answer, when it was asked and for which backend (its address).
    radio_ok: Option<(bool, Instant, usize)>,
    /// Radio songs found ahead for the last song of the queue (its key;
    /// `None` while they are still being found).
    radio_ready: Option<(String, Option<Vec<Track>>)>,
    /// The list the queue was started from (its listening counts for the sidebar).
    playing_from: Option<ListKey>,
    /// Volume the sleep fade set last (a different one was set by hand).
    fade_last: Option<f32>,
    // sleep timer
    sleep: Option<Sleep>,
    fade_from: Option<f32>,
    /// The timer paused playback: the volume comes back when it plays again.
    await_pause: bool,
    /// Pause as soon as the song is loaded (`true`: also back to its start).
    pause_when_loaded: Option<bool>,
    /// "End of song" ended: the next song waits paused at its start.
    after_song: bool,
    // statistics
    stats_pending: Option<(Track, u64)>,
    stats_tick: Option<Instant>,
    stats_flushed: Instant,
    activity_recorded: bool,
}

pub struct Discover {
    inner: Mutex<Inner>,
}

impl Discover {
    fn g(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Called once at start-up from `glyph_app::setup`.
pub fn setup(core: &Arc<Core>) {
    // Asks the backend whether radio can deliver (probe_radio): once it is
    // up and again whenever the backend was rebuilt.
    let probe_core = core.clone();
    crate::rt::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let Some(b) = probe_core.backend() else { continue };
            let ptr = Arc::as_ptr(&b) as usize;
            let Some(d) = probe_core.discover() else { continue };
            if d.g().radio_ok.is_some_and(|(_, at, p)| p == ptr && at.elapsed() < RADIO_PROBE_TTL) {
                continue;
            }
            let ok = b.radio_available().await.unwrap_or(false);
            d.g().radio_ok = Some((ok, Instant::now(), ptr));
        }
    });
    let (playing_from, activity_recorded) = {
        let st = core.st();
        (st.playing_from.clone(), st.listen_counted)
    };
    core.set_discover(Discover {
        inner: Mutex::new(Inner {
            radio_on: true,
            drift: None,
            drift_loading: false,
            radio_ok: None,
            radio_ready: None,
            playing_from,
            fade_last: None,
            sleep: None,
            fade_from: None,
            await_pause: false,
            pause_when_loaded: None,
            after_song: false,
            stats_pending: None,
            stats_tick: None,
            stats_flushed: Instant::now(),
            activity_recorded,
        }),
    });
}

// ---- small helpers ----

fn toast(core: &Core, key: &str, vars: Option<Value>, sub: Option<&str>, kind: &str) {
    core.emit("toast", json!({ "key": key, "text": key, "vars": vars, "sub": sub, "kind": kind }));
}

/// Text of an i18n key in the user's language (English when missing).
fn tr(lang: &str, key: &str, vars: &[(&str, &str)]) -> String {
    let file = match lang {
        "de" => include_str!("../../../assets/i18n/de.toml"),
        "ru" => include_str!("../../../assets/i18n/ru.toml"),
        "es" => include_str!("../../../assets/i18n/es.toml"),
        "fr" => include_str!("../../../assets/i18n/fr.toml"),
        "pt" => include_str!("../../../assets/i18n/pt.toml"),
        "it" => include_str!("../../../assets/i18n/it.toml"),
        "pl" => include_str!("../../../assets/i18n/pl.toml"),
        "tr" => include_str!("../../../assets/i18n/tr.toml"),
        "uk" => include_str!("../../../assets/i18n/uk.toml"),
        "af" => include_str!("../../../assets/i18n/af.toml"),
        _ => include_str!("../../../assets/i18n/en.toml"),
    };
    let find = |src: &str| -> Option<String> {
        let prefix = format!("\"{key}\" = \"");
        let line = src.lines().find(|l| l.starts_with(&prefix))?;
        let raw = line[prefix.len()..].trim_end().strip_suffix('"')?;
        Some(raw.replace("\\\"", "\"").replace("\\n", "\n").replace("\\\\", "\\"))
    };
    let mut s = find(file).or_else(|| find(include_str!("../../../assets/i18n/en.toml"))).unwrap_or_else(|| key.to_string());
    for (k, v) in vars {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

fn lang(core: &Core) -> &'static str {
    core.st().config.resolved_language()
}

fn copy(core: &Core, text: &str) -> bool {
    match core.host.write_clipboard(text) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(error = %e, "clipboard");
            false
        }
    }
}

fn hide_explicit(core: &Core) -> bool {
    core.st().config.audio.hide_explicit
}

/// Songs of a list and its title (menus.rs list_tracks / list_title).
fn list_tracks(core: &Core, l: &ListKey) -> (Vec<Track>, String) {
    let lg = lang(core);
    let st = core.st();
    match l {
        ListKey::Liked => (st.library.liked.clone(), tr(lg, "library.liked", &[])),
        ListKey::Recent => (st.library.recent_tracks().cloned().collect(), tr(lg, "library.recent", &[])),
        ListKey::Playlist { id } => match st.library.playlist(id) {
            Some(p) => (p.tracks.clone(), p.name.clone()),
            None => (Vec::new(), String::new()),
        },
        ListKey::Album { title, tracks, .. } => (tracks.clone(), title.clone()),
        ListKey::ArtistLiked { artist } => (
            st.library.liked.iter().filter(|t| by_artist(&t.artist, artist)).cloned().collect(),
            tr(lg, "library.liked", &[]),
        ),
        ListKey::Downloads => {
            drop(st);
            (core.backend().map(|b| b.downloaded_tracks()).unwrap_or_default(), tr(lg, "library.downloads", &[]))
        }
    }
}

// ---- radio (playback.rs) ----

/// One artist never plays twice in a row.
fn spread_artists(mut rest: Vec<Track>) -> Vec<Track> {
    let mut out: Vec<Track> = Vec::with_capacity(rest.len());
    while !rest.is_empty() {
        let last = out.last().map(|t| primary_artist(&t.artist).to_lowercase());
        let i = rest.iter().position(|t| Some(primary_artist(&t.artist).to_lowercase()) != last).unwrap_or(0);
        out.push(rest.remove(i));
    }
    out
}

/// Song radio: ranks `found` by how close each song sounds to `seed` (its
/// genres and moods). The listener's overall taste only nudges the order.
async fn focus_on_song(core: &Arc<Core>, b: &glyph_core::Backend, seed: &Track, mut found: Vec<(Track, f32)>) -> Vec<Track> {
    // Genres come from the artists' tags: fetch the ones Glyph does not know yet.
    let (missing, use_server, enrich, taste_on) = {
        let st = core.st();
        let mut seen = HashSet::new();
        let missing: Vec<String> = std::iter::once(seed)
            .chain(found.iter().map(|(t, _)| t))
            .map(|t| primary_artist(&t.artist))
            .filter(|a| {
                let k = normalize_artist(a);
                !k.is_empty() && seen.insert(k.clone()) && !st.library.taste.artist_tags.contains_key(&k)
            })
            .take(30)
            .collect();
        let cfg = &st.config.taste;
        (missing, cfg.server_collab, cfg.metadata_enrichment, cfg.enabled)
    };
    if enrich && !missing.is_empty() {
        if let Ok(Ok(items)) = tokio::time::timeout(Duration::from_secs(6), b.taste_tags(missing, use_server)).await {
            if !items.is_empty() {
                core.st().library.taste.merge_artist_tags(&items, glyph_core::paths::now_unix());
            }
        }
    }
    let st = core.st();
    let taste = glyph_core::taste::Taste::from_library(&st.library);
    drop(st);
    let vibe = glyph_core::drift::Vibe::of_tracks(std::slice::from_ref(seed), &taste);
    let bucket = glyph_core::taste::TimeBucket::now();
    for (t, w) in &mut found {
        let fit = vibe.fit(t, &taste);
        let mut s = *w + fit * 2.0;
        // Known genres, none close to the song's: another kind of music.
        if !vibe.genres.is_empty() && fit <= 0.05 && !taste.track_tags(t).0.is_empty() {
            s -= 1.2;
        }
        if taste_on {
            let personal = taste.score_recommendation(t, *w, bucket, false);
            if !personal.is_finite() {
                *w = f32::NEG_INFINITY;
                continue;
            }
            s += (personal - *w) * 0.3;
        }
        *w = s;
    }
    found.retain(|(_, w)| w.is_finite());
    found.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut per_artist: HashMap<String, usize> = HashMap::new();
    let top: Vec<Track> = found
        .into_iter()
        .map(|(t, _)| t)
        .filter(|t| {
            let n = per_artist.entry(normalize_artist(&primary_artist(&t.artist))).or_default();
            *n += 1;
            *n <= 3
        })
        .take(50)
        .collect();
    spread_artists(top)
}

/// Picks up to five seeds, spread over the list.
fn pick_seeds(seeds: &[Track]) -> Vec<Track> {
    const SEEDS: usize = 5;
    let step = (seeds.len() as f32 / SEEDS as f32).max(1.0);
    (0..SEEDS.min(seeds.len())).map(|i| seeds[(i as f32 * step) as usize].clone()).collect()
}

/// Drops hidden artists, disliked songs and (when asked) explicit songs.
fn drop_unwanted(core: &Core, found: &mut Vec<(Track, f32)>) {
    let hide = hide_explicit(core);
    let st = core.st();
    found.retain(|(t, _)| {
        !st.library.is_hidden(&t.artist) && !st.library.taste.is_disliked_track(t) && !(hide && t.maybe_explicit())
    });
}

/// Adds the first `n` songs of `sim` to `found`; higher up the list counts more.
/// A song that is similar to several seeds adds up.
fn add_similar(found: &mut Vec<(Track, f32)>, sim: Vec<Track>, skip: &HashSet<String>, scale: f32, n: usize) {
    for (i, t) in sim.into_iter().take(n).enumerate() {
        let key = track_key(&t);
        if skip.contains(&key) {
            continue;
        }
        let w = (1.0 - i as f32 / (n as f32 + 10.0)) * scale;
        match found.iter_mut().find(|(x, _)| track_key(x) == key) {
            Some(f) => f.1 += w,
            None => found.push((t, w)),
        }
    }
}

/// Songs similar to `seeds`. `deep` (song radio) also asks for the songs
/// similar to the best few matches, so the station has far more to pick from.
async fn similar_pool(b: &glyph_core::Backend, seeds: &[Track], skip: &HashSet<String>, deep: bool) -> (Vec<(Track, f32)>, Option<String>) {
    let mut found: Vec<(Track, f32)> = Vec::new();
    let mut error = None;
    let n = if deep { 60 } else { 40 };
    for answer in futures::future::join_all(seeds.iter().map(|s| b.similar(s))).await {
        match answer {
            Ok(sim) => add_similar(&mut found, sim, skip, 1.0, n),
            Err(e) => error = Some(e.to_string()),
        }
    }
    if deep && !found.is_empty() {
        let mut top = found.clone();
        top.sort_by(|a, b| b.1.total_cmp(&a.1));
        top.truncate(5);
        let more = tokio::time::timeout(
            Duration::from_secs(8),
            futures::future::join_all(top.iter().map(|(t, _)| b.similar(t))),
        )
        .await
        .unwrap_or_default();
        for sim in more.into_iter().flatten() {
            add_similar(&mut found, sim, skip, 0.55, 40);
        }
    }
    (found, error)
}

/// The songs of a radio station from `picked` seeds. One seed (song radio)
/// keeps the station close to that song's sound.
async fn radio_tracks(core: &Arc<Core>, b: &glyph_core::Backend, picked: &[Track], skip: &HashSet<String>) -> (Vec<Track>, Option<String>) {
    let focus = picked.len() == 1;
    let server_radio = {
        let st = core.st();
        st.config.taste.is_server_mode() && b.server_taste_engine_available()
    };

    // Server-first Radio (default ON): uses server intent, collaborative graph,
    // cached artist top tracks & MMR diversity in a single round-trip.
    let mut server_found: Vec<(Track, f32)> = Vec::new();
    if server_radio {
        let seed_reqs: Vec<glyph_core::taste::TasteSeedTrack> = picked
            .iter()
            .enumerate()
            .map(|(i, s)| glyph_core::taste::TasteSeedTrack {
                title: s.title.clone(),
                artist: s.artist.clone(),
                album: s.album.clone(),
                weight: (1.0 - i as f32 * 0.1).max(0.5),
                is_session: true,
            })
            .collect();
        let exclude_song_keys: Vec<String> = picked.iter().map(glyph_core::taste::song_key).collect();
        if let Some(resp) = crate::taste::run_server_recommend(
            core,
            glyph_core::taste::TastePurpose::Radio,
            seed_reqs,
            &exclude_song_keys,
            Some(45),
        )
        .await
        {
            let recs: Vec<Track> = resp.recommendations.into_iter().map(|r| r.track).filter(|t| !skip.contains(&track_key(t))).collect();
            let n = recs.len().max(1) as f32;
            server_found = recs.into_iter().enumerate().map(|(i, t)| (t, 1.2 - i as f32 / n)).collect();
            drop_unwanted(core, &mut server_found);
        }
    }

    // Song radio: the server's songs and a deep search of similar songs
    // together, then sorted by how close they sound to the song.
    if focus {
        let (mut found, error) = similar_pool(b, picked, skip, true).await;
        for (t, w) in server_found {
            let key = track_key(&t);
            match found.iter_mut().find(|(x, _)| track_key(x) == key) {
                Some(f) => f.1 += w,
                None => found.push((t, w)),
            }
        }
        drop_unwanted(core, &mut found);
        return (focus_on_song(core, b, &picked[0], found).await, error);
    }
    if !server_found.is_empty() {
        return (server_found.into_iter().map(|(t, _)| t).collect(), None);
    }

    // Local fallback (local-only opt-out, offline, or server returned empty).
    let (mut found, error) = similar_pool(b, picked, skip, false).await;
    drop_unwanted(core, &mut found);
    let st = core.st();
    let taste_on = st.config.taste.enabled;
    let taste = if taste_on {
        glyph_core::taste::Taste::from_library(&st.library)
    } else {
        glyph_core::taste::Taste::default()
    };
    drop(st);
    let bucket = glyph_core::taste::TimeBucket::now();
    if taste_on {
        // The station should sound like its seeds, not only like the
        // listener's overall taste.
        let vibe = glyph_core::drift::Vibe::of_tracks(picked, &taste);
        for (t, w) in &mut found {
            *w = taste.score_recommendation(t, *w, bucket, false) + vibe.fit(t, &taste) * 0.8;
        }
        found.retain(|(_, w)| w.is_finite());
    }
    found.sort_by(|a, b| b.1.total_cmp(&a.1));
    let tracks = if taste_on {
        taste.mmr_rank_tracks(&found, 50, 3)
    } else {
        spread_artists(found.into_iter().map(|(t, _)| t).take(60).collect())
    };
    (tracks, error)
}

/// Plays the radio songs. `append`: they go on after the ended queue
/// (`ended_on`) instead of replacing it.
fn finish_radio(core: &Arc<Core>, tracks: Vec<Track>, error: Option<String>, title: &str, append: bool, ended_on: Option<String>) {
    let label = tr(lang(core), "queue.radio_from", &[("title", title)]);
    match (tracks.is_empty(), error) {
        (false, _) if append => {
            // Something else started meanwhile: that wins.
            let first = {
                let mut st = core.st();
                st.radio_loading = false;
                let still_ended = st.current.as_ref().map(track_key) == ended_on && st.queue.peek_next().is_none();
                if still_ended {
                    st.queue.append(tracks);
                    let f = st.queue.skip_next().cloned();
                    if f.is_some() {
                        st.context = Some(label);
                    }
                    f
                } else {
                    None
                }
            };
            if let Some(t) = first {
                core.start_track(t);
            } else {
                core.emit_player();
            }
        }
        (false, _) => core.play_list(tracks, 0, Some(label)),
        (true, e) => {
            let err_msg = e.as_deref().unwrap_or("radio_empty").to_string();
            if append {
                let mut st = core.st();
                st.radio_loading = false;
                if st.current.as_ref().map(track_key) == ended_on && st.queue.peek_next().is_none() {
                    st.error = Some(err_msg.clone());
                }
            }
            if let Some(e) = e {
                toast(core, "toast.radio_error", Some(json!({ "e": e })), None, "info");
            } else {
                toast(core, "toast.radio_empty", None, None, "info");
            }
            if append {
                core.emit_player();
            }
        }
    }
}

/// Plays songs similar to `seeds` (up to five, spread over the list).
/// `append`: the songs go on after the ended queue instead of replacing it.
/// One seed (song radio) keeps the station close to that song's sound.
fn radio_from_seeds(core: &Arc<Core>, seeds: Vec<Track>, exclude: HashSet<String>, title: String, append: bool) {
    let Some(b) = core.backend() else { return };
    if seeds.is_empty() {
        toast(core, "toast.radio_empty", None, None, "info");
        return;
    }
    let picked = pick_seeds(&seeds);
    let mut skip = exclude;
    skip.extend(seeds.iter().map(track_key));
    let ended_on = core.st().current.as_ref().map(track_key);
    if append {
        let mut st = core.st();
        st.radio_loading = true;
        st.error = None;
        drop(st);
        core.emit_player();
    }
    toast(core, "toast.radio", Some(json!({ "title": title })), None, "ok");
    let core = core.clone();
    crate::rt::spawn(async move {
        let (tracks, error) = radio_tracks(&core, &b, &picked, &skip).await;
        finish_radio(&core, tracks, error, &title, append, ended_on);
    });
}

/// Seeds and songs to leave out for the radio after the queue ends on `current`.
fn end_seeds(current: &Track, played: &[Track]) -> (Vec<Track>, HashSet<String>) {
    let exclude = played.iter().map(track_key).collect();
    let seeds = std::iter::once(current.clone()).chain(played.iter().rev().take(4).cloned()).collect();
    (seeds, exclude)
}

/// The last song of the queue started: finds its radio songs now, in the
/// background, so the radio starts right away when the song ends.
fn prefetch_radio(core: &Arc<Core>, d: &Discover) {
    let (current, played) = {
        let st = core.st();
        if st.queue.peek_next().is_some() || st.config.taste.is_drift() {
            return;
        }
        (st.current.clone(), st.queue.history().into_iter().cloned().collect::<Vec<Track>>())
    };
    let Some(current) = current else { return };
    if !d.g().radio_on || crate::storage::is_offline() {
        return;
    }
    let Some(b) = core.backend() else { return };
    let key = track_key(&current);
    {
        let mut g = d.g();
        if g.radio_ready.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        g.radio_ready = Some((key.clone(), None));
    }
    let (seeds, mut skip) = end_seeds(&current, &played);
    skip.extend(seeds.iter().map(track_key));
    let picked = pick_seeds(&seeds);
    let core = core.clone();
    crate::rt::spawn(async move {
        let (tracks, _) = radio_tracks(&core, &b, &picked, &skip).await;
        let Some(d) = core.discover() else { return };
        let mut g = d.g();
        if g.radio_ready.as_ref().is_some_and(|(k, _)| *k == key) {
            g.radio_ready = Some((key, Some(tracks)));
        }
    });
}

/// Hook: the queue ended and nothing follows. Returns true when this took
/// over (radio started, or "end of song" ended the timer), so the caller
/// must not show "End of queue". Call from `Core::next` where it finds no
/// next song and `auto` is true.
pub fn on_queue_end(core: &Arc<Core>) -> bool {
    let Some(d) = core.discover() else { return false };
    let radio = {
        let mut g = d.g();
        if g.sleep == Some(Sleep::EndOfSong) {
            // The queue ended anyway: nothing more to stop.
            g.sleep = None;
            drop(g);
            core.emit("sleep", json!({ "state": "done" }));
            return true;
        }
        g.radio_on
    };
    if !radio || crate::storage::is_offline() || core.backend().is_none() {
        return false;
    }
    let (current, played) = {
        let st = core.st();
        (st.current.clone(), st.queue.history().into_iter().cloned().collect::<Vec<Track>>())
    };
    let Some(current) = current else { return false };
    if core.st().config.taste.is_drift() {
        drift_start(core, current, played);
        return true;
    }
    // Found in the background while the song played: start right away.
    let key = track_key(&current);
    let ready = match d.g().radio_ready.take() {
        Some((k, Some(v))) if k == key && !v.is_empty() => Some(v),
        _ => None,
    };
    if let Some(mut tracks) = ready {
        // Disliked or hidden while the song played.
        let mut scored: Vec<(Track, f32)> = tracks.drain(..).map(|t| (t, 0.0)).collect();
        drop_unwanted(core, &mut scored);
        if !scored.is_empty() {
            toast(core, "toast.radio", Some(json!({ "title": current.title })), None, "ok");
            finish_radio(core, scored.into_iter().map(|(t, _)| t).collect(), None, &current.title, true, Some(key));
            return true;
        }
    }
    let (seeds, exclude) = end_seeds(&current, &played);
    radio_from_seeds(core, seeds, exclude, current.title, true);
    true
}

// ---- Glyph Drift Radio (beta) ----

/// Starts a Drift station after the queue ended on `current`.
fn drift_start(core: &Arc<Core>, current: Track, played: Vec<Track>) {
    let Some(d) = core.discover() else { return };
    let lg = lang(core);
    let label = tr(lg, "queue.drift_from", &[("title", &current.title)]);
    let drift = {
        let st = core.st();
        let taste = glyph_core::taste::Taste::from_library(&st.library);
        let seeds: Vec<Track> = std::iter::once(current.clone()).chain(played.iter().rev().take(4).cloned()).collect();
        glyph_core::drift::Drift::new(label, &seeds, played.iter().map(track_key), &taste, glyph_core::paths::now_unix())
    };
    {
        let mut g = d.g();
        g.drift = Some(drift);
        g.drift_loading = false;
    }
    toast(core, "toast.drift", Some(json!({ "title": current.title })), None, "ok");
    drift_refill(core, true);
}

/// Songs of one artist for Drift: the server's top songs, else the songs of
/// the newest release (Deezer). Never Last.fm.
async fn drift_artist_songs(b: &glyph_core::Backend, name: &str, lang: &str) -> Vec<Track> {
    let top = tokio::time::timeout(Duration::from_secs(5), b.artist_server(name, lang)).await.ok().flatten();
    let mut songs: Vec<Track> = top.map(|s| s.top.into_iter().take(6).collect()).unwrap_or_default();
    if songs.is_empty() {
        if let Ok(Ok(releases)) = tokio::time::timeout(Duration::from_secs(5), b.artist_releases(name)).await {
            if let Some(r) = releases.first() {
                if let Ok(Ok(t)) = tokio::time::timeout(Duration::from_secs(5), b.release_tracks(r)).await {
                    songs = t.into_iter().take(5).collect();
                }
            }
        }
    }
    songs.retain(|t| by_artist(&t.artist, name));
    songs
}

/// Adds the next few Drift songs. `ended`: the queue already ran out, so the
/// first new song starts right away.
fn drift_refill(core: &Arc<Core>, ended: bool) {
    let Some(d) = core.discover() else { return };
    let Some(b) = core.backend() else { return };
    {
        let mut g = d.g();
        if g.drift.is_none() || g.drift_loading {
            return;
        }
        g.drift_loading = true;
    }
    let ended_on = core.st().current.as_ref().map(track_key);
    if ended {
        let mut st = core.st();
        st.radio_loading = true;
        st.error = None;
        drop(st);
        core.emit_player();
    }
    let core = core.clone();
    crate::rt::spawn(async move {
        use glyph_core::drift::BATCH;
        let Some(d) = core.discover() else { return };
        let lg = lang(&core);
        let hide = hide_explicit(&core);
        // What the listener did since the last refill steers the station.
        let (taste, events, server_mode) = {
            let st = core.st();
            (
                glyph_core::taste::Taste::from_library(&st.library),
                st.library.taste.session_events.clone(),
                st.config.taste.is_server_mode() && b.server_taste_engine_available(),
            )
        };
        let plan = {
            let mut g = d.g();
            g.drift.as_mut().map(|dr| {
                dr.absorb(&events, &taste);
                (dr.seeds(4), dr.next_artists(&taste, 6), dr.label.clone(), dr.vibe.clone())
            })
        };
        let Some((seeds, artists, label, vibe)) = plan else {
            d.g().drift_loading = false;
            return;
        };

        // 1. Glyph's own server engine (collaborative graph, no Last.fm).
        let server = async {
            if !server_mode || seeds.is_empty() {
                return Vec::new();
            }
            let reqs: Vec<glyph_core::taste::TasteSeedTrack> = seeds
                .iter()
                .enumerate()
                .map(|(i, s)| glyph_core::taste::TasteSeedTrack {
                    title: s.title.clone(),
                    artist: s.artist.clone(),
                    album: s.album.clone(),
                    weight: (1.0 - i as f32 * 0.15).max(0.4),
                    is_session: true,
                })
                .collect();
            let exclude: Vec<String> = seeds.iter().map(glyph_core::taste::song_key).collect();
            crate::taste::run_server_recommend(&core, glyph_core::taste::TastePurpose::Radio, reqs, &exclude, Some(30))
                .await
                .map(|r| r.recommendations.into_iter().enumerate().map(|(i, r)| (r.track, 1.0 - i as f32 / 40.0)).collect())
                .unwrap_or_default()
        };
        // 2. Neighbours in Glyph's artist graph: their top songs.
        let graph = futures::future::join_all(artists.iter().map(|(a, w)| {
            let b = b.clone();
            let name = taste.display_artist(a);
            let w = *w;
            async move {
                drift_artist_songs(&b, &name, lg)
                    .await
                    .into_iter()
                    .enumerate()
                    .map(|(i, t)| (t, (w.min(1.5) * 0.7 + 0.3) - i as f32 * 0.08))
                    .collect::<Vec<_>>()
            }
        }));
        let (server, graph) = futures::join!(server, graph);
        let mut cands: Vec<(Track, f32)> = server;
        cands.extend(graph.into_iter().flatten());

        // 3. Thin pool: the listener's own songs that fit the vibe.
        if cands.len() < BATCH * 3 {
            let st = core.st();
            let mut seen = HashSet::new();
            for t in st.library.liked.iter().chain(st.library.playlists.iter().flat_map(|p| p.tracks.iter())).take(800) {
                if seen.insert(track_key(t)) && vibe.fit(t, &taste) > 0.3 {
                    cands.push((t.clone(), -0.4));
                }
            }
        }
        // 4. Last resort only: the classic similar songs of the newest seed.
        if cands.len() < BATCH {
            if let Some(s) = seeds.first() {
                if let Ok(v) = b.similar(s).await {
                    cands.extend(v.into_iter().take(25).enumerate().map(|(i, t)| (t, 0.4 - i as f32 / 60.0)));
                }
            }
        }

        let tracks = {
            let st = core.st();
            cands.retain(|(t, _)| !st.library.is_hidden(&t.artist) && !st.library.taste.is_disliked_track(t) && !(hide && t.maybe_explicit()));
            drop(st);
            let bucket = glyph_core::taste::TimeBucket::now();
            let mut g = d.g();
            g.drift_loading = false;
            match g.drift.as_mut() {
                Some(dr) if dr.label == label => dr.pick(cands, &taste, bucket, BATCH),
                _ => Vec::new(),
            }
        };

        if tracks.is_empty() {
            if ended {
                {
                    let mut st = core.st();
                    st.radio_loading = false;
                }
                d.g().drift = None;
                toast(&core, "toast.radio_empty", None, None, "info");
                core.emit_player();
            }
            return;
        }
        if ended {
            let first = {
                let mut st = core.st();
                st.radio_loading = false;
                let still_ended = st.current.as_ref().map(track_key) == ended_on && st.queue.peek_next().is_none();
                if still_ended {
                    st.queue.append(tracks);
                    let f = st.queue.skip_next().cloned();
                    if f.is_some() {
                        st.context = Some(label);
                    }
                    f
                } else {
                    None
                }
            };
            match first {
                Some(t) => core.start_track(t),
                None => {
                    d.g().drift = None;
                    core.emit_player();
                }
            }
        } else {
            let added = {
                let mut st = core.st();
                if st.context.as_deref() == Some(label.as_str()) {
                    st.queue.append(tracks);
                    true
                } else {
                    false
                }
            };
            if added {
                core.emit_queue();
            }
        }
    });
}

/// Keeps a Drift station going: refills when few songs are left and stops
/// once the listener plays something else.
fn drift_on_track_started(core: &Arc<Core>, d: &Discover) {
    let label = match d.g().drift.as_ref() {
        Some(dr) => dr.label.clone(),
        None => return,
    };
    let (same, left, on) = {
        let st = core.st();
        (st.context.as_deref() == Some(label.as_str()), st.queue.upcoming().len(), st.config.taste.is_drift())
    };
    if !same || !on {
        d.g().drift = None;
        return;
    }
    if left <= glyph_core::drift::LOW_WATER && !crate::storage::is_offline() {
        drift_refill(core, false);
    }
}

pub fn set_radio(core: C<'_>, on: bool) {
    if let Some(d) = core.discover() {
        d.g().radio_on = on;
    }
}

pub async fn radio_state(core: C<'_>) -> Res<Value> {
    let d = core.discover().ok_or_else(|| "discover not ready".to_string())?;
    let (on, cached) = {
        let g = d.g();
        (g.radio_on, g.radio_ok)
    };
    let ok = match core.backend() {
        Some(b) => {
            let ptr = Arc::as_ptr(&b) as usize;
            match cached.filter(|(_, at, p)| *p == ptr && at.elapsed() < RADIO_PROBE_TTL) {
                Some((ok, _, _)) => Some(ok),
                None => {
                    let ok = b.radio_available().await.unwrap_or(false);
                    d.g().radio_ok = Some((ok, Instant::now(), ptr));
                    Some(ok)
                }
            }
        }
        // Unknown instead of "broken": the switch stays usable.
        None => None,
    };
    Ok(json!({ "on": on, "ok": ok }))
}

pub fn start_radio_tracks(core: &Arc<Core>, seeds: Vec<Track>, title: String) {
    radio_from_seeds(core, seeds, HashSet::new(), title, false);
}

pub fn start_list_radio(core: &Arc<Core>, list: &ListKey) {
    let (tracks, title) = list_tracks(core, list);
    radio_from_seeds(core, tracks, HashSet::new(), title, false);
}

pub fn list_radio(core: C, list: ListKey) {
    start_list_radio(core, &list);
}

pub fn artist_radio(core: C, artist: String, page_tracks: Option<Vec<Track>>) {
    // The artist's page first, then what Glyph already knows: liked and
    // recently played songs, then the followed entry.
    let mut seeds: Vec<Track> = page_tracks.unwrap_or_default().into_iter().take(5).collect();
    if seeds.is_empty() {
        let st = core.st();
        seeds = st.library.liked.iter().chain(st.library.recent_tracks()).filter(|t| by_artist(&t.artist, &artist)).take(5).cloned().collect();
    }
    if seeds.is_empty() {
        let st = core.st();
        seeds.extend(st.library.followed.iter().find(|f| f.name.eq_ignore_ascii_case(&artist)).and_then(|f| f.track.clone()));
    }
    if seeds.is_empty() {
        toast(&core, "toast.radio_empty", None, None, "info");
        return;
    }
    radio_from_seeds(core, seeds, HashSet::new(), artist, false);
}

pub fn song_radio(core: C, track: Track, title: String) {
    radio_from_seeds(core, vec![track], HashSet::new(), title, false);
}

/// "Add songs" panel: songs similar to the playlist's own, minus what is in it.
pub async fn add_suggestions(core: C<'_>, id: String) -> Res<Vec<Track>> {
    let b = core.backend_or_err()?;
    let (own, liked, hide) = {
        let st = core.st();
        let own = st.library.playlist(&id).map(|p| p.tracks.clone()).unwrap_or_default();
        (own, st.library.liked.clone(), st.config.audio.hide_explicit)
    };
    let pool = if own.is_empty() { liked } else { own.clone() };
    let n = pool.len();
    let k = n.min(5);
    let mut seeds: Vec<Track> = (0..k).map(|i| pool[n - 1 - i * n / k].clone()).collect();
    seeds.dedup_by(|a, b| a.id == b.id && a.source == b.source);
    if seeds.is_empty() {
        // Nothing to go on: what Home recommends.
        let seed = {
            let st = core.st();
            let t = st.library.recent_tracks().next().cloned().or_else(|| st.library.liked.first().cloned());
            t
        };
        let recs = match seed {
            Some(t) => b.similar(&t).await.unwrap_or_default(),
            None => Vec::new(),
        };
        let trending = b.trending(20).await.unwrap_or_default();
        return Ok(recs.into_iter().chain(trending).collect());
    }
    // Glyph's own graph: artists next to the playlist's artists (shared
    // playlists, co-listening, cached similar artists) add their top songs.
    let (taste_on, taste, lg) = {
        let st = core.st();
        let on = st.config.taste.enabled;
        let t = if on { glyph_core::taste::Taste::from_library(&st.library) } else { glyph_core::taste::Taste::default() };
        (on, t, st.config.resolved_language())
    };
    let vibe = glyph_core::drift::Vibe::of_tracks(&pool, &taste);
    let neighbours: Vec<String> = if taste_on {
        let mut near: HashMap<String, f32> = HashMap::new();
        for (a, w) in &vibe.artists {
            for (n, s) in taste.artist_links(a).iter().take(8) {
                if !vibe.artists.contains_key(n) {
                    *near.entry(n.clone()).or_default() += w * s;
                }
            }
        }
        let mut v: Vec<(String, f32)> = near.into_iter().filter(|(a, _)| taste.artist_affinity(a) > -0.2 && !taste.is_hidden(a)).collect();
        v.sort_by(|x, y| y.1.total_cmp(&x.1).then_with(|| x.0.cmp(&y.0)));
        v.into_iter().take(4).map(|(a, _)| taste.display_artist(&a)).collect()
    } else {
        Vec::new()
    };
    let near_songs = futures::future::join_all(neighbours.iter().map(|a| {
        let b = b.clone();
        let a = a.clone();
        async move {
            tokio::time::timeout(Duration::from_secs(4), b.artist_server(&a, lg))
                .await
                .ok()
                .flatten()
                .map(|s| s.top.into_iter().take(5).collect::<Vec<Track>>())
                .unwrap_or_default()
        }
    }));
    let (answers, near_songs) = futures::join!(futures::future::join_all(seeds.iter().map(|s| b.similar(s))), near_songs);
    let mut found: Vec<(Track, f32)> = Vec::new();
    for (i, t) in near_songs.into_iter().flat_map(|v| v.into_iter().enumerate()) {
        let w = 0.8 - i as f32 * 0.1;
        let key = track_key(&t);
        match found.iter_mut().find(|(x, _)| track_key(x) == key) {
            Some(f) => f.1 += w,
            None => found.push((t, w)),
        }
    }
    for sim in answers {
        for (i, t) in sim.unwrap_or_default().into_iter().take(30).enumerate() {
            let w = 1.0 - i as f32 / 40.0;
            let key = track_key(&t);
            match found.iter_mut().find(|(x, _)| track_key(x) == key) {
                Some(f) => f.1 += w,
                None => found.push((t, w)),
            }
        }
    }
    let have: HashSet<String> = own.iter().map(track_key).collect();
    let st = core.st();
    let bucket = glyph_core::taste::TimeBucket::now();
    if taste_on {
        // A playlist has its own sound: songs that fit it beat songs that
        // only fit the listener's overall taste.
        for (t, w) in &mut found {
            let fit = glyph_core::drift::playlist_fit(&vibe, t, &taste);
            *w = taste.score_recommendation(t, *w, bucket, false) * 0.6 + fit * 1.4;
        }
        found.retain(|(_, w)| w.is_finite());
    }
    found.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut per: HashMap<String, usize> = HashMap::new();
    Ok(found
        .into_iter()
        .map(|(t, _)| t)
        .filter(|t| {
            !(hide && t.maybe_explicit())
                && !st.library.is_hidden(&t.artist)
                && !st.library.taste.is_disliked_track(t)
                && !have.contains(&track_key(t))
        })
        .filter(|t| {
            let n = per.entry(normalize_artist(&t.artist)).or_default();
            *n += 1;
            *n <= 3
        })
        .take(50)
        .collect())
}

pub fn recommend_feedback(core: C, artist: String, preference: Option<i8>) {
    let key = artist.trim().to_lowercase();
    let norm = normalize_artist(&artist);
    {
        let mut st = core.st();
        match preference {
            Some(p) => {
                st.library.recommendation_feedback.insert(key, p);
            }
            None => {
                st.library.recommendation_feedback.retain(|k, _| {
                    k != &key && (norm.is_empty() || normalize_artist(k) != norm)
                });
                if !norm.is_empty() {
                    st.library.artist_skips.remove(&norm);
                    let remove_sig = if let Some(sig) = st.library.taste.artist_signals.get_mut(&norm) {
                        sig.instant_skips = 0;
                        sig.early_skips = 0;
                        sig.mid_abandons = 0;
                        sig.consecutive_skips = 0;
                        sig.plays == 0 && sig.completions == 0
                    } else {
                        false
                    };
                    if remove_sig {
                        st.library.taste.artist_signals.remove(&norm);
                    }
                }
            }
        }
    }
    core.library_changed();
    // Home's recommendations follow the new liking.
    crate::commands::reload_feeds(core);
}

// ---- lyrics extras (lyrics.rs) ----

pub async fn lyrics_alternatives(core: C<'_>, track: Track) -> Res<Vec<Value>> {
    let b = core.backend_or_err()?;
    let offset = core.st().library.lyrics_offset(&track);
    let alts = b.lyrics_candidates(&track).await.map_err(err)?;
    Ok(alts.iter().map(|l| dto::lyrics(l, offset)).collect())
}

/// Keeps the chosen version for this song. The UI sends the wire shape; the
/// real candidate (with its id) is found again before it is pinned.
pub async fn lyrics_pick(core: C<'_>, track: Track, lyrics: Value) -> Res {
    let b = core.backend_or_err()?;
    let source = lyrics["source"].as_str().unwrap_or_default().to_string();
    let synced = lyrics["synced"].as_bool().unwrap_or(false);
    let lines: Vec<(Option<u64>, String)> = lyrics["lines"]
        .as_array()
        .map(|a| a.iter().map(|l| (l["time_ms"].as_u64(), l["text"].as_str().unwrap_or_default().to_string())).collect())
        .unwrap_or_default();
    let alts = b.lyrics_candidates(&track).await.map_err(err)?;
    let hit: Option<Lyrics> = alts.into_iter().find(|c| {
        let n = if c.synced.is_empty() { c.plain.as_deref().unwrap_or_default().lines().count() } else { c.synced.len() };
        let first = c.synced.first().map(|l| l.text.clone()).or_else(|| c.plain.as_deref().and_then(|p| p.lines().next().map(str::to_string)));
        c.provider == source && !c.synced.is_empty() == synced && n == lines.len() && first == lines.first().map(|l| l.1.clone())
    });
    if let Some(l) = hit {
        b.pin_lyrics(&track, &l);
    }
    Ok(())
}

// ---- statistics (ui/stats.rs) ----

fn top_artists_json(s: &stats::MonthStats, n: usize) -> Vec<Value> {
    let tracks = stats::top_tracks(s, usize::MAX);
    stats::top_artists(s, n)
        .into_iter()
        .map(|(name, a)| {
            let t = tracks.iter().find(|x| by_artist(&x.track.artist, &name)).map(|x| x.track.clone());
            json!({ "name": name, "ms": a.ms, "plays": a.plays, "track": t })
        })
        .collect()
}

fn top_tracks_json(s: &stats::MonthStats, n: usize) -> Vec<Value> {
    stats::top_tracks(s, n).into_iter().map(|x| json!({ "track": x.track, "plays": x.plays, "ms": x.ms })).collect()
}

fn padded(mut v: Vec<u64>, n: usize) -> Vec<u64> {
    v.resize(n, 0);
    v
}

fn months_json(months: Vec<(String, u64)>) -> Vec<Value> {
    months.into_iter().map(|(m, ms)| json!({ "month": m, "ms": ms })).collect()
}

pub fn stats_summary(core: C, range: String) -> Value {
    let r = match range.as_str() {
        "last_month" => Range::LastMonth,
        "year" => Range::Year,
        "all" => Range::All,
        _ => Range::ThisMonth,
    };
    let st = core.st();
    let s = st.library.stats.summary(r, When::now());
    let months = st.library.stats.by_month();
    let months: Vec<(String, u64)> = months.into_iter().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect();
    json!({
        "range": range, "ms": s.ms, "plays": s.plays, "skips": s.skips,
        "artists": s.artists.len(), "songs": s.tracks.len(),
        "active_days": s.days.iter().filter(|d| **d > 0).count(),
        "top_artists": top_artists_json(&s, 10), "top_tracks": top_tracks_json(&s, 10),
        "hours": padded(s.hours.clone(), 24), "weekdays": padded(s.weekdays.clone(), 7),
        "months": months_json(months), "years": st.library.stats.years(),
    })
}

pub fn stats_recap(core: C, year: String) -> Value {
    let st = core.st();
    let s = st.library.stats.year(&year);
    let months: Vec<(String, u64)> = st.library.stats.by_month().into_iter().filter(|(m, _)| m.starts_with(year.as_str())).collect();
    json!({
        "year": year, "ms": s.ms, "plays": s.plays, "artists": s.artists.len(), "songs": s.tracks.len(),
        "top_artists": top_artists_json(&s, 5), "top_tracks": top_tracks_json(&s, 50),
        "hours": padded(s.hours.clone(), 24), "weekdays": padded(s.weekdays.clone(), 7),
        "months": months_json(months),
    })
}

/// Hook: the statistics switch changed (Settings > Playback > Content). Call
/// from `set_config` when `config.stats.enabled` differs from before. Off
/// deletes what was counted.
pub fn on_stats_toggle(core: &Arc<Core>, on: bool) {
    if let Some(d) = core.discover() {
        d.g().stats_pending = None;
    }
    if !on {
        core.st().library.stats = Default::default();
        core.library_changed();
        toast(core, "stats.deleted", None, None, "info");
    }
}

/// Time counted so far is written now (an output change, signing out).
pub fn flush_stats(core: &Arc<Core>) {
    let Some(d) = core.discover() else { return };
    let pending = d.g().stats_pending.take();
    if let Some((t, ms)) = pending {
        add_time(core, &t, ms);
    }
}

/// Signing out: what was counted belongs to the old library.
pub fn reset_stats_pending(core: &Arc<Core>) {
    if let Some(d) = core.discover() {
        let (playing_from, listen_counted) = {
            let st = core.st();
            (st.playing_from.clone(), st.listen_counted)
        };
        let mut g = d.g();
        g.stats_pending = None;
        g.playing_from = playing_from;
        g.activity_recorded = listen_counted;
    }
}

/// The UI starts a queue from an open list (`null`: from elsewhere): playing
/// it counts a little for the sidebar, listening from it more.
pub fn set_playing_from(core: C, list: Option<ListKey>) {
    if let Some(l) = &list {
        core.st().library.record_list_use(&l, 0.5, glyph_core::paths::now_unix());
        core.library_changed();
    }
    core.st().playing_from = list.clone();
    if let Some(d) = core.discover() {
        d.g().playing_from = list;
    }
    core.playback_changed();
}

/// Hook: a song was skipped within 30 s (next to `library.record_skip`).
pub fn on_skip(core: &Arc<Core>) {
    let mut st = core.st();
    if st.config.stats.enabled {
        st.library.stats.add_skip(When::now());
        drop(st);
        core.library_changed();
    }
}

// ---- sharing (library.rs, lists.rs) ----

fn glyph_links(core: &Core) -> bool {
    core.st().config.sharing.glyph_links && core.backend().is_some_and(|b| b.account().is_some())
}

/// Copies a Glyph link of `tracks` and says so; false when it failed.
async fn copy_glyph_link(core: &Core, title: &str, tracks: &[Track], kind: &str, quiet: bool) -> String {
    let Some(b) = core.backend() else { return String::new() };
    let text = b.share_text(title, tracks, kind, None).await;
    if glyph_core::share::link_id(&text).is_some() {
        if !quiet {
            copy(core, &text);
            toast(core, "toast.share_link_copied", None, Some(title), "ok");
        }
        text
    } else {
        toast(core, "toast.share_link_failed", None, None, "info");
        String::new()
    }
}

/// Copies a short link (public server) or a share code for `tracks`.
/// `quiet` only returns it (the phone share menu copies or sends it itself).
async fn share_songs_inner(core: &Core, title: String, tracks: Vec<Track>, kind: &str, origin: Option<String>, quiet: bool) -> String {
    // Glyph links turned off in Settings > Account: always the code.
    let b = core.backend().filter(|_| core.st().config.sharing.glyph_links);
    let Some(b) = b else {
        let code = glyph_core::share::encode(&title, &tracks);
        if !quiet {
            copy(core, &code);
            toast(core, "toast.share_copied", None, Some(&title), "ok");
        }
        return code;
    };
    let signed_in = b.account().is_some();
    let text = b.share_text(&title, &tracks, kind, origin).await;
    let is_link = glyph_core::share::link_id(&text).is_some();
    if signed_in && !is_link {
        toast(core, "toast.share_link_failed", None, None, "info");
        return String::new();
    }
    if !quiet {
        copy(core, &text);
        toast(core, if is_link { "toast.share_link_copied" } else { "toast.share_copied" }, None, Some(&title), "ok");
    }
    text
}

/// `copy: Some(false)` returns the link without copying it or saying so.
pub async fn share_songs(core: C<'_>, title: String, tracks: Vec<Track>, kind: Option<String>, copy: Option<bool>) -> Res<String> {
    Ok(share_songs_inner(&core, title, tracks, kind.as_deref().unwrap_or("playlist"), None, copy == Some(false)).await)
}

/// `copy: Some(false)` returns the link without copying it or saying so.
pub async fn share(core: C<'_>, track: Option<Track>, list: Option<ListKey>, copy: Option<bool>) -> Res<String> {
    let quiet = copy == Some(false);
    if let Some(t) = track {
        if glyph_links(&core) {
            return Ok(copy_glyph_link(&core, &t.title, std::slice::from_ref(&t), "track", quiet).await);
        }
        return Ok(match &t.permalink {
            Some(url) => {
                if !quiet {
                    self::copy(&core, url);
                    toast(&core, "toast.link_copied_plain", None, Some(&t.title), "ok");
                }
                url.clone()
            }
            None => {
                toast(&core, "toast.no_link", None, None, "info");
                String::new()
            }
        });
    }
    let Some(l) = list else { return Ok(String::new()) };
    let (tracks, title) = list_tracks(&core, &l);
    if tracks.is_empty() {
        toast(&core, "toast.share_empty", None, None, "info");
        return Ok(String::new());
    }
    let kind = if matches!(l, ListKey::Album { .. }) { "album" } else { "playlist" };
    let origin = match &l {
        ListKey::Playlist { id } => core.st().library.playlist(id).and_then(|p| p.origin.clone()),
        _ => None,
    };
    Ok(share_songs_inner(&core, title, tracks, kind, origin, quiet).await)
}

/// A few songs of `name`: known ones first, else a short search.
async fn artist_tracks(core: &Core, name: &str) -> Vec<Track> {
    let known: Vec<Track> = {
        let st = core.st();
        st.library.liked.iter().chain(st.library.recent_tracks()).filter(|t| by_artist(&t.artist, name)).take(20).cloned().collect()
    };
    if !known.is_empty() {
        return known;
    }
    let Some(b) = core.backend() else { return Vec::new() };
    let mut stream = b.search(name);
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut out = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok(Some(ev)) = tokio::time::timeout(left, stream.next()).await else { break };
        match ev {
            SearchEvent::Results { tracks, .. } => {
                out.extend(tracks.into_iter().filter(|t| by_artist(&t.artist, name)));
                if out.len() >= 10 {
                    break;
                }
            }
            SearchEvent::Done => break,
            _ => {}
        }
    }
    out.truncate(20);
    out
}

/// `copy: Some(false)` returns the link without copying it or saying so
/// (the phone share menu copies or sends it itself).
pub async fn copy_artist_link(core: C<'_>, artist: String, copy: Option<bool>) -> Res<String> {
    let quiet = copy == Some(false);
    let tracks = artist_tracks(&core, &artist).await;
    if glyph_links(&core) {
        return Ok(copy_glyph_link(&core, &artist, &tracks, "artist", quiet).await);
    }
    let lg = core.st().config.language.clone();
    let wiki = match core.backend() {
        Some(b) => b.artist_info(&artist, &lg).await.ok().flatten().map(|i| i.url).filter(|u| !u.is_empty()),
        None => None,
    };
    match wiki.or_else(|| tracks.iter().find_map(|t| t.permalink.clone())) {
        Some(u) => {
            if !quiet {
                self::copy(&core, &u);
                toast(&core, "toast.link_copied_plain", None, Some(&artist), "ok");
            }
            Ok(u)
        }
        None => {
            toast(&core, "toast.no_link", None, None, "info");
            Ok(String::new())
        }
    }
}

// ---- small settings commands ----

pub fn hidden_artists(core: C) -> Vec<String> {
    core.st().library.hidden_artists.clone()
}

/// The UI shows its own toast.
pub fn toggle_hidden_artist(core: C, name: String) {
    core.st().library.toggle_hidden(&name);
    core.library_changed();
}

pub fn onboarding_seen() {
    let _ = glyph_core::paths::write_atomic(&glyph_core::paths::data_dir().join("onboarding-seen"), b"1");
}

pub fn providers_cooling(core: C) -> Vec<Value> {
    let Some(b) = core.backend() else { return Vec::new() };
    ProviderId::ALL.into_iter().filter(|p| b.provider_cooling_down(*p)).filter_map(|p| serde_json::to_value(p).ok()).collect()
}

pub fn playback_status() -> Value {
    // The device and MPD code lives in jam.rs and knows the live state.
    crate::jam::playback_status()
}

// ---- sleep timer (sleep.rs) ----

pub fn set_sleep_timer(core: &Core, mins: Option<u32>, end_of_song: bool) {
    let Some(d) = core.discover() else { return };
    let restore = {
        let mut g = d.g();
        g.after_song = false;
        g.await_pause = false;
        g.sleep = if end_of_song {
            Some(Sleep::EndOfSong)
        } else {
            mins.filter(|m| *m > 0).map(|m| Sleep::At(Instant::now() + Duration::from_secs(m as u64 * 60)))
        };
        g.fade_from.take()
    };
    if let (Some(v), Some(p)) = (restore, &core.player) {
        p.set_volume(v);
    }
    if end_of_song {
        // The next song must not start gaplessly after this one.
        if let Some(p) = &core.player {
            p.drop_next();
        }
    }
}

pub fn sleep_status(core: &Core) -> Value {
    let Some(d) = core.discover() else {
        return json!({ "active": false, "remaining_s": Value::Null, "end_of_song": false });
    };
    let s = d.g().sleep;
    match s {
        Some(Sleep::At(until)) => {
            let secs = until.saturating_duration_since(Instant::now()).as_secs();
            json!({ "active": true, "remaining_s": secs, "end_of_song": false })
        }
        Some(Sleep::EndOfSong) => json!({ "active": true, "remaining_s": Value::Null, "end_of_song": true }),
        None => json!({ "active": false, "remaining_s": Value::Null, "end_of_song": false }),
    }
}

/// Pause after `mins` minutes or at the end of the song; null / false = off.
/// The UI shows its own toasts.
pub fn set_sleep(core: C, mins: Option<u32>, end_of_song: bool) {
    set_sleep_timer(core, mins, end_of_song);
}

/// Hook: true while "end of song" is set; the caller must not preload the
/// next song (NearEnd).
pub fn sleep_blocks_preload(core: &Arc<Core>) -> bool {
    core.discover().is_some_and(|d| d.g().sleep == Some(Sleep::EndOfSong))
}

/// Hook: a song ended on its own (`PlayerEvent::Ended`). Call BEFORE
/// `core.next(true)`. With "end of song" set, the next song starts and waits
/// paused at its start.
pub fn on_song_ended(core: &Arc<Core>) {
    {
        let mut st = core.st();
        if st.config.taste.enabled && !st.outcome_recorded {
            if let Some(cur) = st.current.clone() {
                let dur = cur.duration_ms.unwrap_or(180_000);
                let hour = glyph_core::taste::local_hour();
                let is_weekend = glyph_core::taste::local_is_weekend();
                let now_s = glyph_core::paths::now_unix();
                let ctx = crate::taste::play_context_from_state(
                    st.playing_from.as_ref(),
                    st.context.as_deref(),
                    st.queue.current_is_manual(),
                );
                let repeat_one = st.queue.repeat() == glyph_core::RepeatMode::One;
                st.library.taste.record_playback_outcome_ctx(
                    &cur,
                    dur,
                    cur.duration_ms,
                    false,
                    true,
                    ctx,
                    hour,
                    is_weekend,
                    now_s,
                );
                if repeat_one {
                    st.library.taste.record_intent_action(
                        &cur,
                        glyph_core::taste::IntentAction::Replayed,
                        ctx,
                        now_s,
                    );
                }
                st.outcome_recorded = true;
                drop(st);
                core.library_changed_soon();
            }
        }
    }
    let Some(d) = core.discover() else { return };
    let has_next = core.st().queue.peek_next().is_some();
    let mut g = d.g();
    if g.sleep == Some(Sleep::EndOfSong) && has_next {
        g.sleep = None;
        g.after_song = true;
    }
}

/// Hook: a song was started (`Core::start_track`). Call with the new track.
pub fn on_track_started(core: &Arc<Core>, _track: &Track) {
    let Some(d) = core.discover() else { return };
    let flush = {
        let mut g = d.g();
        g.activity_recorded = false;
        if g.after_song {
            g.after_song = false;
            g.pause_when_loaded = Some(true);
        }
        g.stats_pending.take()
    };
    if let Some((t, ms)) = flush {
        add_time(core, &t, ms);
    }
    drift_on_track_started(core, &d);
    prefetch_radio(core, &d);
}

fn add_time(core: &Arc<Core>, t: &Track, ms: u64) {
    if ms == 0 {
        return;
    }
    core.st().library.stats.add_time(t, ms, When::now());
    core.library_changed();
}

/// Hook: call every pump tick (100 ms). Counts listening time and plays for
/// the statistics and runs the sleep timer with its fade-out.
pub fn on_tick(core: &Arc<Core>) {
    let Some(d) = core.discover() else { return };
    let ps = core.player_state();
    let (cur, resolving, stats_on, muted) = {
        let st = core.st();
        (st.current.clone(), st.resolving, st.config.stats.enabled, st.muted_volume.is_some())
    };
    let playing = ps.as_ref().is_some_and(|s| s.status == Status::Playing) && !resolving;
    tick_stats(core, &d, ps.as_ref(), cur.as_ref(), playing, stats_on);
    tick_sleep(core, &d, ps.as_ref(), resolving, playing, muted);
}

fn tick_stats(core: &Arc<Core>, d: &Discover, ps: Option<&glyph_audio::PlayerState>, cur: Option<&Track>, playing: bool, on: bool) {
    let now = Instant::now();
    let mut flush: Vec<(Track, u64)> = Vec::new();
    let mut listen: Option<Track> = None;
    {
        let mut g = d.g();
        let last = g.stats_tick.replace(now);
        if !on {
            g.stats_pending = None;
        } else if let (true, Some(cur), Some(last)) = (playing, cur, last) {
            // Long gaps (sleep, a stalled frame) are not listening.
            let dt = now.duration_since(last).min(Duration::from_millis(500)).as_millis() as u64;
            let key = track_key(cur);
            match &mut g.stats_pending {
                Some((t, ms)) if track_key(t) == key => *ms += dt,
                _ => {
                    flush.extend(g.stats_pending.take());
                    g.stats_pending = Some((cur.clone(), dt));
                }
            }
        }
        if on && g.stats_flushed.elapsed() > Duration::from_secs(15) {
            g.stats_flushed = now;
            flush.extend(g.stats_pending.take());
        }
        // A play counts after half the song (5 to 30 s).
        if playing && !g.activity_recorded {
            if let (Some(track), Some(ps)) = (cur, ps) {
                let threshold = track.duration_ms.map(|ms| (ms / 2000).clamp(5, 30)).unwrap_or(30);
                if ps.position.as_secs() >= threshold && ps.track.as_ref().is_some_and(|t| same_song(t, track)) {
                    g.activity_recorded = true;
                    listen = Some(track.clone());
                }
            }
        }
    }
    for (t, ms) in flush {
        add_time(core, &t, ms);
    }
    if let Some(t) = listen {
        {
            let mut st = core.st();
            st.listen_counted = true;
            st.library.record_listen(&t, glyph_core::paths::now_unix());
            let from = core.discover().and_then(|d| d.g().playing_from.clone());
            if let Some(l) = from {
                st.library.record_list_use(&l, 1.0, glyph_core::paths::now_unix());
            }
            if st.config.stats.enabled {
                st.library.stats.add_play(&t, When::now());
            }
        }
        core.playback_pos_changed();
        core.library_changed();
    }
}

fn tick_sleep(core: &Arc<Core>, d: &Discover, ps: Option<&glyph_audio::PlayerState>, resolving: bool, playing: bool, muted: bool) {
    let Some(p) = &core.player else { return };
    let status_playing = ps.is_some_and(|s| s.status == Status::Playing);
    let mut pause = false;
    let mut seek0 = false;
    let mut done = false;
    // Only this computer's player fades; devices and jams just pause.
    let local = !crate::account::controlling(core) && !crate::jam::remote_on() && !crate::jam::jam_active();
    {
        let mut g = d.g();
        // A song that was still loading when the timer ran out.
        if let Some(back) = g.pause_when_loaded {
            if !resolving && status_playing {
                g.pause_when_loaded = None;
                pause = true;
                seek0 = back;
                done = back;
            }
        }
        if let Some(Sleep::At(until)) = g.sleep {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                g.sleep = None;
                g.await_pause = g.fade_from.is_some();
                g.fade_last = None;
                done = true;
                if resolving {
                    // The new song loads paused.
                    crate::jam::set_hold(true);
                } else if status_playing {
                    pause = true;
                }
            } else if left < FADE && playing && !muted && local {
                // The volume comes back when playback starts again.
                // A level set by hand meanwhile is the new one to fade from.
                if let (Some(last), Some(s)) = (g.fade_last, ps) {
                    if (s.volume - last).abs() > 0.05 {
                        g.fade_from = Some(s.volume);
                    }
                }
                let from = *g.fade_from.get_or_insert_with(|| ps.map_or(1.0, |s| s.volume));
                let v = from * left.as_secs_f32() / FADE.as_secs_f32();
                g.fade_last = Some(v);
                p.set_volume(v);
            }
        }
        if !matches!(g.sleep, Some(Sleep::At(_))) && !pause {
            if let Some(from) = g.fade_from {
                if g.await_pause {
                    if !status_playing {
                        g.await_pause = false;
                    }
                } else if status_playing && g.pause_when_loaded.is_none() {
                    g.fade_from = None;
                    p.set_volume(from);
                }
            }
        }
    }
    if pause {
        if local {
            p.pause();
        } else {
            // MPD, a device or the jam: pause there too.
            core.toggle_play();
        }
        if seek0 {
            p.seek(Duration::ZERO);
        }
        core.emit_player();
    }
    if done {
        core.emit("sleep", json!({ "state": "done" }));
    }
}
