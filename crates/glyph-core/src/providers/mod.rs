//! All metadata / playback providers and the registry that builds them from
//! the config. Providers without a key are skipped.

pub mod archive;
pub mod audius;
pub mod deezer;
pub mod jamendo;
pub mod lastfm;
pub mod lrclib;
pub mod musicbrainz;
pub mod soundcloud;
pub mod spotify;
pub mod youtube;

use crate::config::Config;
use crate::provider::MusicProvider;
use glyph_protocol::{Capabilities, ProviderId};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Providers taking part in the unified search (MusicBrainz, Last.fm and
/// LRCLIB are enrichment-only).
pub const SEARCHABLE: [ProviderId; 7] = [
    ProviderId::Audius,
    ProviderId::Jamendo,
    ProviderId::Archive,
    ProviderId::SoundCloud,
    ProviderId::YouTube,
    ProviderId::Spotify,
    ProviderId::Deezer,
];

#[derive(Default)]
pub struct Providers {
    all: BTreeMap<ProviderId, Arc<dyn MusicProvider>>,
    pub deezer: Option<Arc<deezer::Deezer>>,
    pub spotify: Option<Arc<spotify::Spotify>>,
    pub youtube: Option<Arc<youtube::YouTube>>,
    pub musicbrainz: Option<Arc<musicbrainz::MusicBrainz>>,
    pub lastfm: Option<Arc<lastfm::LastFm>>,
    pub lrclib: Option<Arc<lrclib::LrcLib>>,
}

impl Providers {
    /// Builds every enabled provider whose key requirements are met.
    pub fn build(config: &Config, http: &reqwest::Client) -> Self {
        let on = |id: ProviderId| config.providers.enabled.contains(&id);
        let mut p = Providers::default();
        let add = |all: &mut BTreeMap<ProviderId, Arc<dyn MusicProvider>>, v: Arc<dyn MusicProvider>| {
            all.insert(v.id(), v);
        };
        let keys = &config.keys;

        if on(ProviderId::Audius) {
            add(&mut p.all, Arc::new(audius::Audius::new(http.clone())));
        }
        if let (true, Some(id)) = (on(ProviderId::Jamendo), &keys.jamendo_client_id) {
            add(&mut p.all, Arc::new(jamendo::Jamendo::new(http.clone(), id.clone())));
        }
        if on(ProviderId::Archive) {
            add(&mut p.all, Arc::new(archive::Archive::new(http.clone())));
        }
        if let (true, Some(id)) = (on(ProviderId::SoundCloud), &keys.soundcloud_client_id) {
            add(&mut p.all, Arc::new(soundcloud::SoundCloud::new(http.clone(), id.clone())));
        }
        if let (true, Some(k)) = (on(ProviderId::YouTube), &keys.youtube) {
            let yt = Arc::new(youtube::YouTube::new(http.clone(), k.clone(), config.providers.youtube_daily_searches));
            p.youtube = Some(yt.clone());
            add(&mut p.all, yt);
        }
        if let (true, Some(id), Some(secret)) =
            (on(ProviderId::Spotify), &keys.spotify_client_id, &keys.spotify_client_secret)
        {
            let sp = Arc::new(spotify::Spotify::new(http.clone(), id.clone(), secret.clone()));
            p.spotify = Some(sp.clone());
            add(&mut p.all, sp);
        }
        if on(ProviderId::Deezer) {
            let d = Arc::new(deezer::Deezer::new(http.clone()));
            p.deezer = Some(d.clone());
            add(&mut p.all, d);
        }
        if on(ProviderId::MusicBrainz) {
            let m = Arc::new(musicbrainz::MusicBrainz::new(http.clone()));
            p.musicbrainz = Some(m.clone());
            add(&mut p.all, m);
        }
        if let (true, Some(k)) = (on(ProviderId::LastFm), &keys.lastfm) {
            let l = Arc::new(lastfm::LastFm::new(http.clone(), k.clone()));
            p.lastfm = Some(l.clone());
            add(&mut p.all, l);
        }
        if on(ProviderId::LrcLib) {
            let l = Arc::new(lrclib::LrcLib::new(http.clone()));
            p.lrclib = Some(l.clone());
            add(&mut p.all, l);
        }
        p
    }

    /// Registers (or replaces) a provider. Used by tests and embedders.
    pub fn insert(&mut self, p: Arc<dyn MusicProvider>) {
        self.all.insert(p.id(), p);
    }

    pub fn get(&self, id: ProviderId) -> Option<Arc<dyn MusicProvider>> {
        self.all.get(&id).cloned()
    }

    pub fn ids(&self) -> Vec<ProviderId> {
        self.all.keys().copied().collect()
    }

    /// Providers for the unified search.
    pub fn searchable(&self) -> Vec<Arc<dyn MusicProvider>> {
        SEARCHABLE.iter().filter_map(|id| self.get(*id)).collect()
    }

    /// Providers that can fully play a track.
    pub fn is_full(&self, id: ProviderId) -> bool {
        self.all.get(&id).map(|p| p.capabilities() == Capabilities::FullPlayback).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_without_keys_are_skipped() {
        let http = reqwest::Client::new();
        let cfg = Config::default();
        let p = Providers::build(&cfg, &http);
        let ids = p.ids();
        assert!(ids.contains(&ProviderId::Audius));
        assert!(ids.contains(&ProviderId::Archive));
        assert!(ids.contains(&ProviderId::Deezer));
        assert!(!ids.contains(&ProviderId::Jamendo));
        assert!(!ids.contains(&ProviderId::YouTube));
        assert!(!ids.contains(&ProviderId::SoundCloud));
        assert!(!ids.contains(&ProviderId::Spotify));
        assert!(!ids.contains(&ProviderId::LastFm));

        let mut cfg = Config::default();
        cfg.keys.jamendo_client_id = Some("x".into());
        cfg.keys.youtube = Some("k".into());
        cfg.providers.enabled.retain(|p| *p != ProviderId::Audius);
        let ids = Providers::build(&cfg, &http).ids();
        assert!(ids.contains(&ProviderId::Jamendo) && ids.contains(&ProviderId::YouTube));
        assert!(!ids.contains(&ProviderId::Audius));
    }
}
