//! Listening statistics (opt-in, Settings > Playback > Content).
//!
//! Only kept while the user has turned them on. Each device sums up what it
//! played per calendar month (listening time, plays, per artist, per song,
//! per hour of the day and per weekday). Signed in, every device's months
//! sync as `stats` items keyed `<device>:<YYYY-MM>`; the statistics page adds
//! all devices up. Nothing is sent anywhere else, and turning the switch off
//! deletes them on this device and from the account.

use crate::dedupe;
use glyph_protocol::Track;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Sync kind of a device's month.
pub const SYNC_KIND: &str = "stats";
/// Kept per month: the most-played artists and songs.
const MAX_ARTISTS: usize = 150;
const MAX_TRACKS: usize = 200;
/// Months kept on this device (older ones fold into `earlier`).
const KEEP_MONTHS: usize = 25;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArtistStat {
    pub ms: u64,
    pub plays: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackStat {
    pub track: Track,
    pub ms: u64,
    pub plays: u32,
}

/// One month of one device.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MonthStats {
    /// Listening time.
    pub ms: u64,
    /// Songs listened to past the listening threshold.
    pub plays: u32,
    /// Songs skipped within 30 s.
    #[serde(default)]
    pub skips: u32,
    /// Normalised artist name -> (display name, stats).
    #[serde(default)]
    pub artists: BTreeMap<String, (String, ArtistStat)>,
    /// Song key -> stats.
    #[serde(default)]
    pub tracks: BTreeMap<String, TrackStat>,
    /// Listening time per local hour of the day.
    #[serde(default)]
    pub hours: Vec<u64>,
    /// Listening time per weekday (0 = Monday).
    #[serde(default)]
    pub weekdays: Vec<u64>,
    /// Listening time per day of the month (1-based index - 1).
    #[serde(default)]
    pub days: Vec<u64>,
}

impl MonthStats {
    pub(crate) fn since_guest_import(&self, baseline: &Self) -> Self {
        let mut delta = self.clone();
        delta.ms = delta.ms.saturating_sub(baseline.ms);
        delta.plays = delta.plays.saturating_sub(baseline.plays);
        delta.skips = delta.skips.saturating_sub(baseline.skips);
        for (key, (_, stat)) in &mut delta.artists {
            if let Some((_, old)) = baseline.artists.get(key) { stat.ms = stat.ms.saturating_sub(old.ms); stat.plays = stat.plays.saturating_sub(old.plays); }
        }
        for (key, stat) in &mut delta.tracks {
            if let Some(old) = baseline.tracks.get(key) { stat.ms = stat.ms.saturating_sub(old.ms); stat.plays = stat.plays.saturating_sub(old.plays); }
        }
        for (dst, old) in [(&mut delta.hours, &baseline.hours), (&mut delta.weekdays, &baseline.weekdays), (&mut delta.days, &baseline.days)] {
            for (i, d) in dst.iter_mut().enumerate() { *d = d.saturating_sub(old.get(i).copied().unwrap_or(0)); }
        }
        delta
    }
    pub(crate) fn merge_max(&mut self, other: &Self) {
        self.ms = self.ms.max(other.ms);
        self.plays = self.plays.max(other.plays);
        self.skips = self.skips.max(other.skips);
        for (k, (name, value)) in &other.artists {
            let e = self.artists.entry(k.clone()).or_insert_with(|| (name.clone(), ArtistStat::default()));
            e.1.ms = e.1.ms.max(value.ms); e.1.plays = e.1.plays.max(value.plays);
        }
        for (k, value) in &other.tracks {
            let e = self.tracks.entry(k.clone()).or_insert_with(|| value.clone());
            e.ms = e.ms.max(value.ms); e.plays = e.plays.max(value.plays);
        }
        for (dst, src) in [(&mut self.hours, &other.hours), (&mut self.weekdays, &other.weekdays), (&mut self.days, &other.days)] {
            dst.resize(dst.len().max(src.len()), 0);
            for (d, s) in dst.iter_mut().zip(src) { *d = (*d).max(*s); }
        }
        self.trim();
    }

    pub(crate) fn merge_guest(&mut self, other: &Self) { self.add(other); self.trim(); }
    fn trim(&mut self) {
        if self.artists.len() > MAX_ARTISTS {
            let mut v: Vec<_> = std::mem::take(&mut self.artists).into_iter().collect();
            v.sort_by(|a, b| b.1 .1.ms.cmp(&a.1 .1.ms));
            v.truncate(MAX_ARTISTS);
            self.artists = v.into_iter().collect();
        }
        if self.tracks.len() > MAX_TRACKS {
            let mut v: Vec<_> = std::mem::take(&mut self.tracks).into_iter().collect();
            v.sort_by(|a, b| b.1.ms.cmp(&a.1.ms));
            v.truncate(MAX_TRACKS);
            self.tracks = v.into_iter().collect();
        }
    }

    fn add(&mut self, other: &MonthStats) {
        self.ms += other.ms;
        self.plays += other.plays;
        self.skips += other.skips;
        for (k, (name, a)) in &other.artists {
            let e = self.artists.entry(k.clone()).or_insert_with(|| (name.clone(), ArtistStat::default()));
            e.1.ms += a.ms;
            e.1.plays += a.plays;
        }
        for (k, t) in &other.tracks {
            match self.tracks.get_mut(k) {
                Some(e) => {
                    e.ms += t.ms;
                    e.plays += t.plays;
                }
                None => {
                    self.tracks.insert(k.clone(), t.clone());
                }
            }
        }
        for (dst, src) in [(&mut self.hours, &other.hours), (&mut self.weekdays, &other.weekdays), (&mut self.days, &other.days)] {
            if dst.len() < src.len() {
                dst.resize(src.len(), 0);
            }
            for (d, s) in dst.iter_mut().zip(src) {
                *d += s;
            }
        }
    }
}

/// Local calendar position of a moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct When {
    /// `YYYY-MM`.
    pub month: [u8; 7],
    pub hour: u8,
    pub weekday: u8,
    pub day: u8,
}

impl When {
    pub fn now() -> Self {
        use chrono::{Datelike, Local, Timelike};
        let t = Local::now();
        let m = format!("{:04}-{:02}", t.year(), t.month());
        let mut month = [0u8; 7];
        month.copy_from_slice(&m.as_bytes()[..7]);
        When { month, hour: t.hour() as u8, weekday: t.weekday().num_days_from_monday() as u8, day: t.day() as u8 }
    }

    pub fn month(&self) -> &str {
        std::str::from_utf8(&self.month).unwrap_or("0000-00")
    }
}

/// This device's statistics plus what other devices synced.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    /// `YYYY-MM` -> this device's month.
    #[serde(default)]
    pub months: BTreeMap<String, MonthStats>,
    /// `<device>:<YYYY-MM>` -> another device's month (from sync).
    #[serde(default)]
    pub remote: BTreeMap<String, MonthStats>,
}

/// The time range the statistics page shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    ThisMonth,
    LastMonth,
    Year,
    All,
}

impl Stats {
    pub fn is_empty(&self) -> bool {
        self.months.is_empty() && self.remote.is_empty()
    }

    fn month_mut(&mut self, when: &When) -> &mut MonthStats {
        let m = self.months.entry(when.month().to_string()).or_default();
        if m.hours.len() != 24 {
            m.hours.resize(24, 0);
        }
        if m.weekdays.len() != 7 {
            m.weekdays.resize(7, 0);
        }
        if m.days.len() != 31 {
            m.days.resize(31, 0);
        }
        m
    }

    /// `ms` of listening to `track` (called in small steps while it plays).
    pub fn add_time(&mut self, track: &Track, ms: u64, when: When) {
        if ms == 0 {
            return;
        }
        let m = self.month_mut(&when);
        m.ms += ms;
        m.hours[when.hour as usize % 24] += ms;
        m.weekdays[when.weekday as usize % 7] += ms;
        m.days[(when.day.max(1) as usize - 1) % 31] += ms;
        for a in crate::taste::credited(&track.artist).into_iter().take(3) {
            let display = dedupe::primary_artist(&track.artist);
            let e = m.artists.entry(a).or_insert_with(|| (display, ArtistStat::default()));
            e.1.ms += ms;
        }
        let key = dedupe::track_key(track);
        match m.tracks.get_mut(&key) {
            Some(t) => t.ms += ms,
            None => {
                m.tracks.insert(key, TrackStat { track: track.clone(), ms, plays: 0 });
            }
        }
        m.trim();
        self.prune();
    }

    /// A song counted as listened (past the listening threshold).
    pub fn add_play(&mut self, track: &Track, when: When) {
        let m = self.month_mut(&when);
        m.plays += 1;
        for a in crate::taste::credited(&track.artist).into_iter().take(3) {
            let display = dedupe::primary_artist(&track.artist);
            m.artists.entry(a).or_insert_with(|| (display, ArtistStat::default())).1.plays += 1;
        }
        let key = dedupe::track_key(track);
        m.tracks.entry(key).or_insert_with(|| TrackStat { track: track.clone(), ms: 0, plays: 0 }).plays += 1;
        m.trim();
    }

    pub fn add_skip(&mut self, when: When) {
        self.month_mut(&when).skips += 1;
    }

    fn prune(&mut self) {
        while self.months.len() > KEEP_MONTHS {
            let first = self.months.keys().next().cloned();
            if let Some(k) = first {
                self.months.remove(&k);
            }
        }
    }

    /// Everything of `range` from every device, added up.
    pub fn summary(&self, range: Range, now: When) -> MonthStats {
        let cur = now.month().to_string();
        let last = previous_month(&cur);
        let year = &cur[..4];
        let wanted = |month: &str| match range {
            Range::ThisMonth => month == cur,
            Range::LastMonth => month == last,
            Range::Year => month.starts_with(year),
            Range::All => true,
        };
        let mut out = MonthStats::default();
        for (m, s) in &self.months {
            if wanted(m) {
                out.add(s);
            }
        }
        for (k, s) in &self.remote {
            let month = k.rsplit(':').next().unwrap_or("");
            if wanted(month) {
                out.add(s);
            }
        }
        out
    }

    /// Everything of one year (`"2026"`) from every device, added up.
    pub fn year(&self, year: &str) -> MonthStats {
        let mut out = MonthStats::default();
        let months = self.months.iter().map(|(m, s)| (m.as_str(), s));
        let remote = self.remote.iter().map(|(k, s)| (k.rsplit(':').next().unwrap_or(""), s));
        for (m, s) in months.chain(remote) {
            if m.starts_with(year) {
                out.add(s);
            }
        }
        out
    }

    /// Years with listening time, newest first.
    pub fn years(&self) -> Vec<String> {
        let mut ys: Vec<String> = self.by_month().into_iter().filter(|(_, ms)| *ms > 0).filter_map(|(m, _)| m.get(..4).map(str::to_string)).collect();
        ys.dedup();
        ys.reverse();
        ys
    }

    /// Listening time per month (oldest first), all devices.
    pub fn by_month(&self) -> Vec<(String, u64)> {
        let mut m: BTreeMap<String, u64> = BTreeMap::new();
        for (k, s) in &self.months {
            *m.entry(k.clone()).or_default() += s.ms;
        }
        for (k, s) in &self.remote {
            *m.entry(k.rsplit(':').next().unwrap_or("").to_string()).or_default() += s.ms;
        }
        m.into_iter().filter(|(k, _)| k.len() == 7).collect()
    }

    /// This device's months as sync items: (key, value).
    pub fn sync_items(&self, device: &str) -> Vec<(String, serde_json::Value)> {
        self.months.iter().map(|(m, s)| (format!("{device}:{m}"), serde_json::to_value(s).unwrap_or_default())).collect()
    }

    /// A month synced from the account. Our own device's months are ours.
    pub fn apply_remote(&mut self, device: &str, key: &str, value: Option<&serde_json::Value>) -> bool {
        if key.starts_with(&format!("{device}:")) {
            return false;
        }
        match value.and_then(|v| serde_json::from_value::<MonthStats>(v.clone()).ok()) {
            Some(s) => {
                let changed = self.remote.get(key) != Some(&s);
                self.remote.insert(key.to_string(), s);
                changed
            }
            None => self.remote.remove(key).is_some(),
        }
    }
}

/// `"2026-01"` -> `"2025-12"`.
fn previous_month(m: &str) -> String {
    let y: i32 = m.get(..4).and_then(|s| s.parse().ok()).unwrap_or(1970);
    let mo: i32 = m.get(5..7).and_then(|s| s.parse().ok()).unwrap_or(1);
    if mo <= 1 {
        format!("{:04}-12", y - 1)
    } else {
        format!("{:04}-{:02}", y, mo - 1)
    }
}

/// Top `n` entries by listening time.
pub fn top_artists(s: &MonthStats, n: usize) -> Vec<(String, ArtistStat)> {
    let mut v: Vec<(String, ArtistStat)> = s.artists.values().cloned().collect();
    v.sort_by(|a, b| b.1.ms.cmp(&a.1.ms).then(b.1.plays.cmp(&a.1.plays)));
    v.truncate(n);
    v
}

pub fn top_tracks(s: &MonthStats, n: usize) -> Vec<TrackStat> {
    let mut v: Vec<TrackStat> = s.tracks.values().cloned().collect();
    v.sort_by(|a, b| b.ms.cmp(&a.ms).then(b.plays.cmp(&a.plays)));
    v.truncate(n);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dedupe::test_track;
    use glyph_protocol::ProviderId;

    fn when(month: &str, hour: u8) -> When {
        let mut m = [0u8; 7];
        m.copy_from_slice(month.as_bytes());
        When { month: m, hour, weekday: 2, day: 15 }
    }

    #[test]
    fn year_adds_up_its_months() {
        let mut s = Stats::default();
        let t = test_track(ProviderId::Audius, "1", "Song", "Artist", None);
        s.add_time(&t, 60_000, when("2025-12", 20));
        s.add_time(&t, 30_000, when("2026-01", 8));
        s.add_time(&t, 10_000, when("2026-03", 8));
        assert_eq!(s.year("2026").ms, 40_000);
        assert_eq!(s.year("2025").ms, 60_000);
        assert_eq!(s.years(), vec!["2026".to_string(), "2025".to_string()]);
    }

    #[test]
    fn adds_time_plays_and_sums_devices() {
        let a = test_track(ProviderId::Audius, "1", "Song A", "Artist A", None);
        let b = test_track(ProviderId::Audius, "2", "Song B", "Artist B & Artist A", None);
        let mut s = Stats::default();
        s.add_time(&a, 60_000, when("2026-10", 20));
        s.add_time(&b, 30_000, when("2026-10", 21));
        s.add_play(&a, when("2026-10", 20));
        s.add_time(&a, 10_000, when("2026-09", 8));
        let oct = s.summary(Range::ThisMonth, when("2026-10", 0));
        assert_eq!(oct.ms, 90_000);
        assert_eq!(oct.plays, 1);
        assert_eq!(oct.hours[20], 60_000);
        let top = top_artists(&oct, 5);
        assert_eq!(top[0].0, "Artist A");
        assert_eq!(top[0].1.ms, 90_000);
        assert_eq!(s.summary(Range::LastMonth, when("2026-10", 0)).ms, 10_000);
        assert_eq!(s.summary(Range::All, when("2026-10", 0)).ms, 100_000);

        // Another device's month adds up; our own synced months are ignored.
        let items = s.sync_items("dev1");
        let mut other = Stats::default();
        for (k, v) in &items {
            assert!(other.apply_remote("dev2", k, Some(v)));
        }
        assert!(!other.apply_remote("dev1", &items[0].0, Some(&items[0].1)));
        other.add_time(&a, 5_000, when("2026-10", 9));
        assert_eq!(other.summary(Range::ThisMonth, when("2026-10", 0)).ms, 95_000);
        assert_eq!(other.summary(Range::Year, when("2026-10", 0)).ms, 105_000);
        assert_eq!(other.by_month(), vec![("2026-09".to_string(), 10_000), ("2026-10".to_string(), 95_000)]);
    }

    #[test]
    fn months_roll_over() {
        assert_eq!(previous_month("2026-01"), "2025-12");
        assert_eq!(previous_month("2026-10"), "2026-09");
    }
}
