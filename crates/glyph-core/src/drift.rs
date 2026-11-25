//! Glyph Drift Radio (beta) and the playlist "vibe" used for "Add songs".
//!
//! Drift keeps playing after the queue ends, without asking Last.fm for
//! "similar tracks". It walks Glyph's own artist graph
//! ([`Taste::artist_links`]) from the songs that were just played to the end,
//! and steers with a moving "vibe" (genres and moods): songs that play to the
//! end pull the vibe towards them, skipped songs push it away. Every refill
//! starts from the newest songs, so the station slowly drifts with the
//! listener instead of circling one seed forever.
//!
//! This file is pure logic (no network). The app collects candidates (artist
//! top songs, the listener's own library, the server engine) and asks
//! [`Drift::pick`] for the next few songs.

use crate::dedupe::{normalize_artist, primary_artist, track_key};
use crate::taste::{credited, SessionEvent, SessionEventOutcome, Taste, TimeBucket};
use glyph_protocol::taste::{genre_similarity, mood_similarity};
use glyph_protocol::{ProviderId, Track};
use std::collections::{HashMap, HashSet};

/// Songs added per refill: small, so every refill can follow the listener.
pub const BATCH: usize = 8;
/// Refill when this many songs or fewer are left.
pub const LOW_WATER: usize = 2;
/// Newest completed songs used as seeds.
const MAX_ANCHORS: usize = 6;
/// Old vibe kept per new song (the rest moves towards the new song).
const VIBE_KEEP: f32 = 0.72;
/// The same artist at most this often per refill.
const PER_ARTIST: usize = 2;

/// Weighted genres and moods: what a set of songs sounds like.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Vibe {
    pub genres: HashMap<String, f32>,
    pub moods: HashMap<String, f32>,
    /// Normalized primary artists in it, with how often they appear.
    pub artists: HashMap<String, f32>,
}

impl Vibe {
    /// The vibe of a list of songs (a playlist, the seeds of a station).
    pub fn of_tracks(tracks: &[Track], taste: &Taste) -> Self {
        let mut v = Self::default();
        for t in tracks {
            v.add(t, taste, 1.0);
        }
        v.normalize();
        v
    }

    fn add(&mut self, t: &Track, taste: &Taste, w: f32) {
        let (gs, ms, _) = taste.track_tags(t);
        for g in gs {
            *self.genres.entry(g).or_default() += w;
        }
        for m in ms {
            *self.moods.entry(m).or_default() += w * 0.8;
        }
        let a = normalize_artist(&primary_artist(&t.artist));
        if !a.is_empty() {
            *self.artists.entry(a).or_default() += w.max(0.0);
        }
    }

    /// Scales every map to a largest absolute value of `1.0`.
    fn normalize(&mut self) {
        for m in [&mut self.genres, &mut self.moods, &mut self.artists] {
            let top = m.values().fold(0.0f32, |a, v| a.max(v.abs()));
            if top > 0.0 {
                for v in m.values_mut() {
                    *v /= top;
                }
            }
            m.retain(|_, v| v.abs() > 0.02);
        }
    }

    /// Moves the vibe towards (`w > 0`) or away from (`w < 0`) `t`.
    fn steer(&mut self, t: &Track, taste: &Taste, w: f32) {
        for m in [&mut self.genres, &mut self.moods] {
            for v in m.values_mut() {
                *v *= VIBE_KEEP;
            }
        }
        let (gs, ms, _) = taste.track_tags(t);
        for g in gs {
            let e = self.genres.entry(g).or_default();
            *e = (*e + w * (1.0 - VIBE_KEEP) * 1.6).clamp(-1.0, 1.0);
        }
        for m in ms {
            let e = self.moods.entry(m).or_default();
            *e = (*e + w * (1.0 - VIBE_KEEP) * 1.3).clamp(-1.0, 1.0);
        }
        for m in [&mut self.genres, &mut self.moods] {
            m.retain(|_, v| v.abs() > 0.02);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.genres.is_empty() && self.moods.is_empty()
    }

    /// How well `t` fits this vibe, `-1.0..=1.0` (`0.0` when unknown).
    pub fn fit(&self, t: &Track, taste: &Taste) -> f32 {
        let (gs, ms, _) = taste.track_tags(t);
        let mut sum = 0.0f32;
        let mut n = 0.0f32;
        if !self.genres.is_empty() {
            for g in &gs {
                n += 1.0;
                let (sim, w) = best(&self.genres, |x| genre_similarity(g, x));
                if sim >= 0.55 {
                    sum += w * sim;
                }
            }
        }
        if !self.moods.is_empty() {
            for m in &ms {
                n += 0.5;
                let (sim, w) = best(&self.moods, |x| mood_similarity(m, x));
                if sim >= 0.55 {
                    sum += w * sim * 0.5;
                }
            }
        }
        if n == 0.0 {
            return 0.0;
        }
        (sum / n).clamp(-1.0, 1.0)
    }
}

/// The closest entry of `m` by `sim`: (similarity, its weight).
fn best(m: &HashMap<String, f32>, sim: impl Fn(&str) -> f32) -> (f32, f32) {
    let mut out = (0.0f32, 0.0f32);
    for (k, &w) in m {
        let s = sim(k);
        if s > out.0 || (s == out.0 && w > out.1) {
            out = (s, w);
        }
    }
    out
}

/// Fit of `t` to a playlist: its sound plus its artists and their graph
/// neighbours. Used to rank "Add songs" so suggestions match the playlist,
/// not only the listener's overall taste.
pub fn playlist_fit(vibe: &Vibe, t: &Track, taste: &Taste) -> f32 {
    let mut s = vibe.fit(t, taste) * 1.2;
    let mut near = 0.0f32;
    for a in credited(&t.artist) {
        if let Some(w) = vibe.artists.get(&a) {
            // Already in the playlist: a little, it should not take over.
            near = near.max(0.35 * w.min(1.0));
        }
        for (pa, pw) in &vibe.artists {
            if let Some((_, lw)) = taste.artist_links(pa).iter().find(|(b, _)| *b == a) {
                near = near.max(lw * pw.min(1.0));
            }
        }
    }
    s += near * 0.9;
    s
}

/// One running Drift station.
#[derive(Debug, Clone)]
pub struct Drift {
    /// Queue label ("Glyph Drift · …"); a different one means the user moved on.
    pub label: String,
    pub vibe: Vibe,
    /// Newest completed songs first: the seeds of the next refill.
    pub anchors: Vec<Track>,
    /// Every song already played or queued by this station (`track_key`).
    pub played: HashSet<String>,
    /// Artists skipped during this station (normalized).
    pub avoid: HashSet<String>,
    /// How often each artist was queued by this station.
    pub artist_count: HashMap<String, u32>,
    /// Refills so far. Later refills explore a little further.
    pub steps: u32,
    /// Newest session event already read (its timestamp).
    pub seen_until: u64,
}

impl Drift {
    pub fn new(label: String, seeds: &[Track], played: impl IntoIterator<Item = String>, taste: &Taste, now_s: u64) -> Self {
        let mut played: HashSet<String> = played.into_iter().collect();
        played.extend(seeds.iter().map(track_key));
        Self {
            label,
            vibe: Vibe::of_tracks(seeds, taste),
            anchors: seeds.iter().take(MAX_ANCHORS).cloned().collect(),
            played,
            avoid: HashSet::new(),
            artist_count: HashMap::new(),
            steps: 0,
            seen_until: now_s,
        }
    }

    /// A song ran to the end: it becomes the newest seed and pulls the vibe.
    pub fn finished(&mut self, t: &Track, taste: &Taste) {
        self.vibe.steer(t, taste, 1.0);
        let k = track_key(t);
        self.anchors.retain(|a| track_key(a) != k);
        self.anchors.insert(0, t.clone());
        self.anchors.truncate(MAX_ANCHORS);
        self.played.insert(k);
    }

    /// A song was skipped: its sound and artist are steered away from.
    pub fn skipped(&mut self, t: &Track, taste: &Taste, early: bool) {
        self.vibe.steer(t, taste, if early { -0.8 } else { -0.45 });
        if early {
            self.avoid.extend(credited(&t.artist));
        }
        self.played.insert(track_key(t));
    }

    /// Reads the listener's new session events (completions and skips)
    /// since the last call. `events` is newest first, as stored.
    pub fn absorb(&mut self, events: &[SessionEvent], taste: &Taste) {
        let mut newest = self.seen_until;
        let mut fresh: Vec<&SessionEvent> = events.iter().filter(|e| e.timestamp_s > self.seen_until).collect();
        fresh.reverse();
        for e in fresh {
            newest = newest.max(e.timestamp_s);
            // Only title, artist and album matter for seeds and the vibe.
            let t = Track {
                id: String::new(),
                source: ProviderId::Deezer,
                title: e.title.clone(),
                artist: e.artist.clone(),
                album: e.album.clone(),
                duration_ms: None,
                cover_url: None,
                isrc: None,
                permalink: None,
                external_ids: Default::default(),
                explicit: false,
            };
            match e.outcome {
                SessionEventOutcome::Completed | SessionEventOutcome::Replayed | SessionEventOutcome::Liked => self.finished(&t, taste),
                SessionEventOutcome::SkippedEarly => self.skipped(&t, taste, true),
                SessionEventOutcome::SkippedMid => self.skipped(&t, taste, false),
                _ => {}
            }
        }
        self.seen_until = newest;
    }

    /// Seeds for a refill: the newest completed songs.
    pub fn seeds(&self, n: usize) -> Vec<Track> {
        self.anchors.iter().take(n).cloned().collect()
    }

    /// Artists to look at next: neighbours of the seeds in Glyph's graph,
    /// newer seeds counting more. After a few refills a second hop opens up
    /// so the station keeps moving instead of looping.
    pub fn next_artists(&self, taste: &Taste, n: usize) -> Vec<(String, f32)> {
        let mut score: HashMap<String, f32> = HashMap::new();
        let anchor_keys: Vec<String> = self.anchors.iter().map(|t| normalize_artist(&primary_artist(&t.artist))).collect();
        let hop2 = self.steps >= 2;
        for (i, a) in anchor_keys.iter().enumerate() {
            let w = 1.0 / (1.0 + i as f32 * 0.5);
            for (b, s) in taste.artist_links(a) {
                *score.entry(b.clone()).or_default() += w * s;
                if hop2 {
                    for (c, s2) in taste.artist_links(b).iter().take(6) {
                        *score.entry(c.clone()).or_default() += w * s * s2 * 0.35;
                    }
                }
            }
        }
        // The seeds' own artists stay possible, but not first.
        for a in &anchor_keys {
            *score.entry(a.clone()).or_default() += 0.15;
        }
        let mut v: Vec<(String, f32)> = score
            .into_iter()
            .filter(|(a, _)| !self.avoid.contains(a) && !taste.is_hidden(a) && taste.artist_affinity(a) > -0.2)
            .map(|(a, s)| {
                // Artists this station already played a lot step back.
                let used = self.artist_count.get(&a).copied().unwrap_or(0) as f32;
                let s = s / (1.0 + used * 0.6) + taste.graph_affinity(&a) * 0.2;
                (a, s)
            })
            .collect();
        v.sort_by(|x, y| y.1.total_cmp(&x.1).then_with(|| x.0.cmp(&y.0)));
        v.truncate(n);
        v
    }

    /// Picks the next `n` songs from `cands` (song, how good its source
    /// thinks it is). Marks them as played so the next refill never repeats.
    pub fn pick(&mut self, cands: Vec<(Track, f32)>, taste: &Taste, bucket: TimeBucket, n: usize) -> Vec<Track> {
        let mut seen: HashSet<String> = HashSet::new();
        let explore = (0.15 + self.steps as f32 * 0.04).min(0.4);
        let mut scored: Vec<(Track, f32)> = Vec::new();
        for (t, base) in cands {
            let k = track_key(&t);
            if self.played.contains(&k) || !seen.insert(k) {
                continue;
            }
            if credited(&t.artist).iter().any(|a| self.avoid.contains(a)) {
                continue;
            }
            let familiar = taste.is_liked(&t) || taste.artist_weight(&t.artist) > 0.5;
            let mut s = taste.score_recommendation(&t, base, bucket, familiar);
            if !s.is_finite() {
                continue;
            }
            s += self.vibe.fit(&t, taste) * 1.6;
            // A little room for new artists, growing as the station drifts.
            if taste.artist_weight(&t.artist) < 0.1 {
                s += explore * (0.5 + taste.graph_affinity(&t.artist));
            }
            scored.push((t, s));
        }
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        let ranked = taste.mmr_rank_tracks(&scored, n * 3, PER_ARTIST);
        let mut out = Vec::with_capacity(n);
        let mut per: HashMap<String, usize> = HashMap::new();
        let mut last: Option<String> = self.anchors.first().map(|t| normalize_artist(&primary_artist(&t.artist)));
        for t in ranked {
            let a = normalize_artist(&primary_artist(&t.artist));
            let c = per.entry(a.clone()).or_default();
            if *c >= PER_ARTIST || last.as_deref() == Some(a.as_str()) {
                continue;
            }
            *c += 1;
            last = Some(a.clone());
            *self.artist_count.entry(a).or_default() += 1;
            self.played.insert(track_key(&t));
            out.push(t);
            if out.len() >= n {
                break;
            }
        }
        self.steps += 1;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Library;

    fn tr(title: &str, artist: &str) -> Track {
        Track {
            id: format!("{artist}:{title}"),
            source: ProviderId::Deezer,
            title: title.into(),
            artist: artist.into(),
            album: None,
            duration_ms: Some(200_000),
            cover_url: Some("https://img.example/c.jpg".into()),
            isrc: None,
            permalink: None,
            external_ids: Default::default(),
            explicit: false,
        }
    }

    fn dreamy_library() -> Library {
        let mut lib = Library::default();
        lib.liked.push(tr("Show Me How", "Men I Trust"));
        lib.liked.push(tr("Space Song", "Beach House"));
        lib.create_playlist("Night").tracks =
            vec![tr("Show Me How", "Men I Trust"), tr("Space Song", "Beach House"), tr("Locket", "Crumb")];
        lib
    }

    #[test]
    fn playlist_neighbours_become_graph_links() {
        let lib = dreamy_library();
        let taste = Taste::from_library(&lib);
        assert!(taste.artist_links("men i trust").iter().any(|(b, _)| b == "crumb"));
        assert!(taste.graph_affinity("Crumb") > 0.1);
        assert_eq!(taste.graph_affinity("Nobody Knows Them"), 0.0);
    }

    #[test]
    fn drift_follows_graph_and_avoids_skips() {
        let lib = dreamy_library();
        let taste = Taste::from_library(&lib);
        let seed = tr("Lauren", "Men I Trust");
        let mut d = Drift::new("Drift".into(), std::slice::from_ref(&seed), [], &taste, 0);
        let next = d.next_artists(&taste, 5);
        assert!(next.iter().any(|(a, _)| a == "crumb"), "{next:?}");

        let cands = vec![
            (tr("Bones", "Crumb"), 0.5),
            (tr("Enter Sandman", "Metallica"), 0.5),
            (tr("Myth", "Beach House"), 0.5),
            (tr("Lauren", "Men I Trust"), 0.9),
        ];
        let picked = d.pick(cands.clone(), &taste, TimeBucket::Night, 3);
        assert!(!picked.iter().any(|t| t.title == "Lauren"), "seed must not repeat");
        assert_eq!(picked.first().map(|t| t.artist.as_str()) == Some("Metallica"), false);
        // Nothing repeats on the next refill.
        let again = d.pick(cands, &taste, TimeBucket::Night, 3);
        assert!(again.iter().all(|t| !picked.contains(t)));

        d.skipped(&tr("Bones", "Crumb"), &taste, true);
        assert!(!d.next_artists(&taste, 5).iter().any(|(a, _)| a == "crumb"));
    }

    #[test]
    fn finished_songs_become_seeds_and_move_the_vibe() {
        let taste = Taste::from_library(&dreamy_library());
        let mut d = Drift::new("Drift".into(), &[tr("Show Me How", "Men I Trust")], [], &taste, 0);
        let heavy = tr("Enter Sandman", "Metallica");
        let before = d.vibe.fit(&heavy, &taste);
        for _ in 0..4 {
            d.finished(&heavy, &taste);
        }
        assert_eq!(d.seeds(1)[0].artist, "Metallica");
        assert!(d.vibe.fit(&heavy, &taste) > before);
    }

    #[test]
    fn playlist_fit_prefers_songs_that_sound_like_the_playlist() {
        let lib = dreamy_library();
        let taste = Taste::from_library(&lib);
        let vibe = Vibe::of_tracks(&lib.playlists[0].tracks, &taste);
        let near = playlist_fit(&vibe, &tr("When the Sun Hits", "Slowdive"), &taste);
        let far = playlist_fit(&vibe, &tr("Enter Sandman", "Metallica"), &taste);
        assert!(near > far, "{near} {far}");
        assert!(playlist_fit(&vibe, &tr("Bones", "Crumb"), &taste) > far);
    }
}
