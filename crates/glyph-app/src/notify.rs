//! Phone notifications (Android): "Glyph x.y is ready" and "New from
//! <artist>" for artists you follow. Each kind can be turned off in
//! Settings > Notifications (`config.notifications`) and in Android's own
//! settings (one channel per kind).
//!
//! The check runs while Glyph is open (a minute after the start, then every
//! six hours) and, with the app closed, from Android's job scheduler
//! (`CheckJob.java` -> [`check_headless`]). What was already said is kept in
//! `notified.json` in the data folder, so nothing is said twice. The first
//! check only learns the releases that are already out: following an
//! artist does not bring a flood of old releases.

use crate::controller::Core;
use glyph_core::updater::UpdateStatus;
use glyph_core::{Backend, Config};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Channel of "a new Glyph version" notifications.
pub const CHANNEL_UPDATES: &str = "updates";
/// Channel of "new from an artist you follow" notifications.
pub const CHANNEL_RELEASES: &str = "releases";
/// Releases out longer than this are not news any more.
const RECENT_DAYS: u32 = 7;
/// More new releases than this at once become one summary.
const MAX_SINGLE: usize = 3;
/// Release ids remembered (oldest are forgotten first).
const MAX_SEEN: usize = 600;
const UPDATE_ID: i32 = 1;
const SUMMARY_ID: i32 = 2;
/// While the app is open: first check, then the gap between checks.
const FIRST_CHECK: Duration = Duration::from_secs(60);
const EVERY: Duration = Duration::from_secs(6 * 3600);

/// One notification to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notice {
    /// Same id: replaces the earlier notification.
    pub id: i32,
    pub channel: String,
    /// Name of the channel in Android's settings.
    pub channel_name: String,
    pub title: String,
    pub body: String,
    /// What a tap opens (`glyph://update`, `glyph://album/<artist>/<title>`, `glyph://open`).
    pub url: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Seen {
    /// Version last announced.
    #[serde(default)]
    update: Option<String>,
    /// Release ids already known.
    #[serde(default)]
    releases: Vec<String>,
    /// The first check ran (it only learns what is out already).
    #[serde(default)]
    primed: bool,
}

fn seen_path() -> PathBuf {
    glyph_core::paths::data_dir().join("notified.json")
}

fn load_seen() -> Seen {
    std::fs::read(seen_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_seen(s: &Seen) {
    let path = seen_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(b) = serde_json::to_vec(s) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, b).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

fn tr(lang: &str, key: &str, vars: &[(&str, &str)]) -> String {
    let mut s = crate::i18n::tr_in(lang, key);
    for (k, v) in vars {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A stable notification id per release (3 .. 1 000 002).
fn release_id(id: &str) -> i32 {
    let h = id.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    3 + (h % 1_000_000) as i32
}

/// What Glyph knows now and the user has not been told yet.
struct Found {
    update: Option<String>,
    releases: Vec<glyph_core::providers::deezer::Release>,
}

/// Turns what was found into notices, given what was said before. Updates
/// `seen`.
fn decide(found: Found, seen: &mut Seen, lang: &str, today: &str) -> Vec<Notice> {
    let mut out = Vec::new();
    if let Some(v) = found.update.map(|v| v.trim_start_matches(['v', 'V']).to_string()) {
        if seen.update.as_deref() != Some(v.as_str()) {
            out.push(Notice {
                id: UPDATE_ID,
                channel: CHANNEL_UPDATES.into(),
                channel_name: tr(lang, "notify.channel.updates", &[]),
                title: tr(lang, "notify.update.title", &[("v", &v)]),
                body: tr(lang, "notify.update.body", &[]),
                url: "glyph://update".into(),
            });
            seen.update = Some(v);
        }
    }
    let fresh: Vec<_> = found.releases.into_iter().filter(|r| !seen.releases.iter().any(|x| x == &r.id)).collect();
    if seen.primed {
        // Only real news: out in the last days (a release found late still counts).
        let news: Vec<_> = fresh.iter().filter(|r| r.released.as_deref().is_none_or(|d| days_between(d, today).is_some_and(|n| n <= RECENT_DAYS as i64))).collect();
        let channel_name = tr(lang, "notify.channel.releases", &[]);
        if news.len() > MAX_SINGLE {
            let mut artists: Vec<&str> = Vec::new();
            for r in &news {
                if !artists.iter().any(|a| a.eq_ignore_ascii_case(&r.artist)) {
                    artists.push(&r.artist);
                }
            }
            let n = news.len().to_string();
            out.push(Notice {
                id: SUMMARY_ID,
                channel: CHANNEL_RELEASES.into(),
                channel_name,
                title: tr(lang, "notify.releases.title", &[("n", &n)]),
                body: tr(lang, "notify.releases.body", &[("artists", &artists.iter().take(4).copied().collect::<Vec<_>>().join(", "))]),
                url: "glyph://open".into(),
            });
        } else {
            for r in news {
                // `AlbumKind` is written "Album", "Ep", ...; the keys are lower case.
                let kind = serde_json::to_value(r.kind).ok().and_then(|v| v.as_str().map(str::to_ascii_lowercase)).unwrap_or_else(|| "album".into());
                let kind = tr(lang, &format!("album.kind.{kind}"), &[]);
                out.push(Notice {
                    id: release_id(&r.id),
                    channel: CHANNEL_RELEASES.into(),
                    channel_name: channel_name.clone(),
                    title: tr(lang, "notify.release.title", &[("artist", &r.artist)]),
                    body: tr(lang, "notify.release.body", &[("title", &r.title), ("kind", &kind)]),
                    url: format!("glyph://album/{}/{}", enc(&r.artist), enc(&r.title)),
                });
            }
        }
    }
    seen.releases.extend(fresh.into_iter().map(|r| r.id));
    if seen.releases.len() > MAX_SEEN {
        let cut = seen.releases.len() - MAX_SEEN;
        seen.releases.drain(..cut);
    }
    seen.primed = true;
    out
}

/// Days from `from` to `to` (both `YYYY-MM-DD`).
fn days_between(from: &str, to: &str) -> Option<i64> {
    fn day(s: &str) -> Option<i64> {
        let mut it = s.get(..10)?.split('-');
        let (y, m, d): (i64, i64, i64) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
        // Days since 1970-01-01 (proleptic Gregorian).
        let (y, m) = if m <= 2 { (y - 1, m + 12) } else { (y, m) };
        Some(365 * y + y / 4 - y / 100 + y / 400 + (153 * (m - 3) + 2) / 5 + d - 719_469)
    }
    Some(day(to)? - day(from)?)
}

fn today() -> String {
    let secs = glyph_core::paths::now_unix() as i64;
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

/// One check at a time in this process (the open app and the job).
static RUNNING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Checks with `b` what to say; `followed`: artists you follow, `hidden`:
/// artists you hid (never announced).
pub async fn find(b: &Backend, cfg: &Config, followed: Vec<String>, hidden: &[String]) -> Vec<Notice> {
    let n = &cfg.notifications;
    if !n.app_updates && !n.new_releases {
        return Vec::new();
    }
    let _one = RUNNING.lock().await;
    let update = if n.app_updates {
        match b.check_for_update().await {
            Ok(UpdateStatus::Available(i)) if !i.downgrade => Some(i.version),
            _ => None,
        }
    } else {
        None
    };
    let releases = if n.new_releases && !followed.is_empty() {
        match b.new_releases(followed, RECENT_DAYS * 2).await {
            Ok(list) => list.into_iter().filter(|r| !hidden.iter().any(|h| glyph_core::dedupe::by_artist(&r.artist, h))).collect(),
            Err(e) => {
                tracing::debug!("notify: releases: {e:#}");
                return finish(update, None, cfg);
            }
        }
    } else {
        Vec::new()
    };
    finish(update, Some(releases), cfg)
}

/// `releases: None`: they could not be read (nothing learnt, nothing said).
fn finish(update: Option<String>, releases: Option<Vec<glyph_core::providers::deezer::Release>>, cfg: &Config) -> Vec<Notice> {
    let mut seen = load_seen();
    let primed = seen.primed;
    let out = decide(Found { update, releases: releases.clone().unwrap_or_default() }, &mut seen, cfg.resolved_language(), &today());
    if releases.is_none() {
        seen.primed = primed;
    }
    save_seen(&seen);
    out
}

fn followed_and_hidden(lib: &glyph_core::Library) -> (Vec<String>, Vec<String>) {
    (lib.followed.iter().take(40).map(|a| a.name.clone()).collect(), lib.hidden_artists.clone())
}

/// The check with the running app's backend (the Android job while Glyph is
/// open, and the loop of [`setup`]).
pub async fn check_core(core: &Arc<Core>) -> Vec<Notice> {
    let Some(b) = core.backend() else { return Vec::new() };
    let (cfg, (followed, hidden)) = {
        let st = core.st();
        (st.config.clone(), followed_and_hidden(&st.library))
    };
    find(&b, &cfg, followed, &hidden).await
}

/// The check with Glyph closed (Android job): reads the settings and the
/// library from disk and starts a backend just for this.
pub async fn check_headless() -> Vec<Notice> {
    let Ok(cfg) = Config::load() else { return Vec::new() };
    if !cfg.notifications.app_updates && !cfg.notifications.new_releases {
        return Vec::new();
    }
    let lib = glyph_core::Library::load().unwrap_or_default();
    let (followed, hidden) = followed_and_hidden(&lib);
    let Ok(b) = Backend::new(cfg.clone()).await else { return Vec::new() };
    find(&b, &cfg, followed, &hidden).await
}

/// While the app runs (phones only): checks a minute after the start and
/// then every six hours, and shows what is new.
pub fn setup(core: &Arc<Core>) {
    if !core.host.can_notify() {
        return;
    }
    let core = core.clone();
    crate::rt::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            for n in check_core(&core).await {
                core.host.notify(&n);
            }
            tokio::time::sleep(EVERY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use glyph_core::catalog::AlbumKind;
    use glyph_core::providers::deezer::Release;

    fn rel(id: &str, artist: &str, day: &str) -> Release {
        Release { id: id.into(), title: format!("Album {id}"), artist: artist.into(), cover_url: None, year: Some(2026), released: Some(day.into()), kind: AlbumKind::Album }
    }

    #[test]
    fn first_check_only_learns_then_news_are_told_once() {
        let mut seen = Seen::default();
        let today = "2026-11-20";
        let out = decide(Found { update: None, releases: vec![rel("1", "A", "2026-11-19"), rel("2", "B", "2026-11-01")] }, &mut seen, "en", today);
        assert!(out.is_empty(), "the first check only learns: {out:?}");
        assert!(seen.primed);

        let out = decide(Found { update: Some("v0.13.0".into()), releases: vec![rel("1", "A", "2026-11-19"), rel("3", "C", "2026-11-20"), rel("4", "D", "2026-10-01")] }, &mut seen, "en", today);
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(out[0].channel, CHANNEL_UPDATES);
        assert_eq!(out[0].title, "Glyph 0.13.0 is ready");
        assert_eq!(out[0].url, "glyph://update");
        assert_eq!(out[1].channel, CHANNEL_RELEASES);
        assert_eq!(out[1].title, "New from C");
        assert_eq!(out[1].body, "Album 3 · Album");
        assert_eq!(out[1].url, "glyph://album/C/Album%203");
        // Said once: the same things again stay quiet.
        let out = decide(Found { update: Some("0.13.0".into()), releases: vec![rel("3", "C", "2026-11-20")] }, &mut seen, "en", today);
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn many_new_releases_become_one_summary() {
        let mut seen = Seen { primed: true, ..Default::default() };
        let list = (0..5).map(|i| rel(&format!("r{i}"), if i % 2 == 0 { "Same" } else { "Other" }, "2026-11-20")).collect();
        let out = decide(Found { update: None, releases: list }, &mut seen, "en", "2026-11-20");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, SUMMARY_ID);
        assert_eq!(out[0].title, "5 new releases");
        assert_eq!(out[0].body, "From Same, Other");
        assert_eq!(seen.releases.len(), 5);
    }

    #[test]
    fn dates_and_ids() {
        assert_eq!(days_between("2026-11-13", "2026-11-20"), Some(7));
        assert_eq!(days_between("2024-02-28", "2024-03-01"), Some(2));
        assert_eq!(days_between("2025-12-31", "2026-01-01"), Some(1));
        assert_eq!(days_between("bad", "2026-01-01"), None);
        assert_eq!(today().len(), 10);
        assert_eq!(days_between(&today(), &today()), Some(0));
        let id = release_id("123456");
        assert!(id >= 3 && id == release_id("123456"));
    }
}
