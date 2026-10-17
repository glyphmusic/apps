//! Manual check: `cargo run -p glyph-audio --example embed_smoke -- <youtube-id>`
use glyph_audio::*;
use glyph_protocol::{PlaybackSource, ProviderId, Track};
use std::time::{Duration, Instant};

fn main() {
    let id = std::env::args().nth(1).unwrap_or_else(|| "jNQXAC9IVRw".into());
    let p = PlayerController::new().unwrap();
    let t = Track {
        id: id.clone(), source: ProviderId::YouTube, title: "smoke".into(), artist: "-".into(),
        album: None, duration_ms: None, cover_url: None, isrc: None, permalink: None, external_ids: Default::default(), explicit: false,
    };
    p.load(t, PlaybackSource::Embed { provider: ProviderId::YouTube, embed_id: id });
    let rx = p.events();
    let end = Instant::now() + Duration::from_secs(25);
    while Instant::now() < end {
        if let Ok(e) = rx.recv_timeout(Duration::from_millis(500)) { println!("{e:?}"); }
        let s = p.state();
        println!("{:?} pos={:?} dur={:?}", s.status, s.position, s.duration);
    }
}
