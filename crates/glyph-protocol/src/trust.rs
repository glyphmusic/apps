//! Rules for data that crosses a trust boundary.
//!
//! The app is open source, so whatever a client sends is a claim: the server
//! checks it before it reaches other users. In the other direction, whatever
//! another user sends through the server (a Jam host's queue, a shared
//! playlist, a public profile) is a claim to the receiving app. Server and
//! apps use the same rules from here, so a value passes on both sides or on
//! neither.

use crate::{ProviderId, Track};
use serde::{Deserialize, Deserializer, Serialize};

/// Longest title, artist or album kept from a claim (characters).
pub const MAX_TEXT_CHARS: usize = 300;
/// Longest duration kept from a claim (a long mix, not a broken value).
pub const MAX_DURATION_MS: u64 = 24 * 3600 * 1000;

/// An 11-character YouTube video id. Deserializing anything else fails, so a
/// request with a bad id is refused before handler code sees it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct VideoId(String);

impl VideoId {
    pub fn parse(s: &str) -> Option<Self> {
        valid_video_id(s).then(|| Self(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for VideoId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for VideoId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::parse(&s).ok_or_else(|| serde::de::Error::custom("video_id must be an 11-character YouTube id"))
    }
}

/// `true` for a YouTube video id: exactly 11 of `[A-Za-z0-9_-]`.
pub fn valid_video_id(id: &str) -> bool {
    id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Providers whose ids turn into stream or download requests. Their ids are
/// checked; ids of metadata-only providers are left as they are.
fn plays(p: ProviderId) -> bool {
    matches!(
        p,
        ProviderId::Audius | ProviderId::Jamendo | ProviderId::Archive | ProviderId::SoundCloud | ProviderId::YouTube | ProviderId::Deezer
    )
}

/// A provider-local id that is safe in a URL path segment and has the shape
/// the provider uses.
pub fn plausible_track_id(provider: ProviderId, id: &str) -> bool {
    match provider {
        ProviderId::YouTube => valid_video_id(id),
        ProviderId::SoundCloud | ProviderId::Jamendo => !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit()),
        // Personal uploads have negative ids.
        ProviderId::Deezer => {
            let digits = id.strip_prefix('-').unwrap_or(id);
            !digits.is_empty() && digits.len() <= 20 && digits.bytes().all(|b| b.is_ascii_digit())
        }
        _ => {
            !id.is_empty()
                && id != "."
                && id != ".."
                && id.len() <= 128
                && id.bytes().all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
        }
    }
}

/// Lower-case host of a plain `https://host/...` URL: no user info, no port,
/// no spaces or control characters.
pub(crate) fn https_host(url: &str) -> Option<String> {
    if url.len() > 2048 || url.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return None;
    }
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("").to_ascii_lowercase();
    if host.is_empty() || host.contains(':') || host.contains('@') || host.contains('\\') {
        return None;
    }
    Some(host)
}

fn host_in(host: &str, suffixes: &[&str]) -> bool {
    suffixes.iter().any(|s| host == *s || host.ends_with(&format!(".{s}")))
}

/// An `https` link to a music or reference site Glyph knows. Only such links
/// from claims are shown, copied or followed.
pub fn trusted_link(url: &str) -> bool {
    const SITES: &[&str] = &[
        "youtube.com", "youtu.be", "soundcloud.com", "audius.co", "jamendo.com", "archive.org", "deezer.com",
        "spotify.com", "musicbrainz.org", "last.fm", "wikipedia.org", "lrclib.net", "glyph.my",
    ];
    https_host(url).is_some_and(|h| host_in(&h, SITES))
}

/// Hosts whose images the server's cover cache will fetch. Clients route only
/// these through `/v1/covers/`; everything else is loaded directly.
pub fn cover_cache_allowed(url: &str) -> bool {
    const SUFFIXES: &[&str] = &[
        "dzcdn.net", "deezer.com", "audius.co", "jamendo.com", "archive.org", "sndcdn.com", "ytimg.com",
        "ggpht.com", "scdn.co", "spotifycdn.com", "coverartarchive.org",
    ];
    https_host(url).is_some_and(|h| h == "lastfm.freetls.fastly.net" || host_in(&h, SUFFIXES))
}

/// `true` for an ISRC: two letters, three letters or digits, seven digits.
pub fn valid_isrc(isrc: &str) -> bool {
    let b = isrc.as_bytes();
    b.len() == 12
        && b[..2].iter().all(u8::is_ascii_alphabetic)
        && b[2..5].iter().all(u8::is_ascii_alphanumeric)
        && b[5..].iter().all(u8::is_ascii_digit)
}

fn clamp_text(s: &mut String) {
    if s.chars().any(char::is_control) {
        *s = s.chars().filter(|c| !c.is_control()).collect();
    }
    if s.chars().count() > MAX_TEXT_CHARS {
        *s = s.chars().take(MAX_TEXT_CHARS).collect();
    }
}

impl Track {
    /// Removes what a claim may not carry: covers outside the cover hosts,
    /// links outside [`trusted_link`], malformed ISRCs and ids, control
    /// characters and overlong texts. What stays is still a claim, only a
    /// well-formed one.
    pub fn sanitize(&mut self) {
        clamp_text(&mut self.title);
        clamp_text(&mut self.artist);
        if let Some(album) = self.album.as_mut() {
            clamp_text(album);
        }
        if self.cover_url.as_deref().is_some_and(|u| !cover_cache_allowed(u)) {
            self.cover_url = None;
        }
        if self.permalink.as_deref().is_some_and(|u| !trusted_link(u)) {
            self.permalink = None;
        }
        if let Some(isrc) = self.isrc.as_mut() {
            *isrc = isrc.trim().to_ascii_uppercase();
        }
        if self.isrc.as_deref().is_some_and(|i| !valid_isrc(i)) {
            self.isrc = None;
        }
        if self.duration_ms.is_some_and(|d| d == 0 || d > MAX_DURATION_MS) {
            self.duration_ms = None;
        }
        let source = self.source;
        self.external_ids.retain(|p, id| *p != source && (!plays(*p) || plausible_track_id(*p, id)));
    }

    /// The id this track names at `p` (its own or an `external_ids` entry),
    /// if it has the provider's shape.
    pub fn claimed_id(&self, p: ProviderId) -> Option<&str> {
        let id = if self.source == p { Some(self.id.as_str()) } else { self.external_ids.get(&p).map(String::as_str) };
        id.filter(|id| plausible_track_id(p, id))
    }

    /// `true` if this track names `id` at `p` itself.
    pub fn names_id(&self, p: ProviderId, id: &str) -> bool {
        (self.source == p && self.id == id) || self.external_ids.get(&p).is_some_and(|own| own == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> Track {
        Track {
            id: "123".into(),
            source: ProviderId::Deezer,
            title: "Song".into(),
            artist: "Artist".into(),
            album: None,
            duration_ms: Some(200_000),
            cover_url: Some("https://e-cdns-images.dzcdn.net/images/cover/x/500x500.jpg".into()),
            isrc: Some("gbduw1300001".into()),
            permalink: Some("https://www.deezer.com/track/123".into()),
            external_ids: Default::default(),
            explicit: false,
        }
    }

    #[test]
    fn video_ids_parse_only_when_valid() {
        assert!(VideoId::parse("7wtfhZwyrcc").is_some());
        assert!(VideoId::parse("--exec=abcd").is_none());
        assert!(VideoId::parse("7wtfhZwyrc/").is_none());
        assert!(serde_json::from_str::<VideoId>(r#""7wtfhZwyrcc""#).is_ok());
        assert!(serde_json::from_str::<VideoId>(r#""../../etc""#).is_err());
        assert_eq!(serde_json::to_string(&VideoId::parse("7wtfhZwyrcc").unwrap()).unwrap(), r#""7wtfhZwyrcc""#);
    }

    #[test]
    fn track_ids_follow_the_provider_shape() {
        assert!(plausible_track_id(ProviderId::Audius, "95wro"));
        assert!(plausible_track_id(ProviderId::Archive, "gd1977-05-08.sbd"));
        assert!(plausible_track_id(ProviderId::SoundCloud, "123456"));
        assert!(plausible_track_id(ProviderId::Deezer, "-42"));
        assert!(!plausible_track_id(ProviderId::SoundCloud, "12a"));
        assert!(!plausible_track_id(ProviderId::Audius, "../users/x"));
        assert!(!plausible_track_id(ProviderId::Audius, ".."));
        assert!(!plausible_track_id(ProviderId::Audius, "a?b=c"));
        assert!(!plausible_track_id(ProviderId::YouTube, "short"));
    }

    #[test]
    fn links_and_covers_need_known_https_hosts() {
        assert!(trusted_link("https://www.youtube.com/watch?v=7wtfhZwyrcc"));
        assert!(trusted_link("https://audius.co/Lofibeat/stars-1"));
        assert!(!trusted_link("http://www.youtube.com/watch?v=x"));
        assert!(!trusted_link("https://youtube.com.evil.example/x"));
        assert!(!trusted_link("https://user@soundcloud.com/x"));
        assert!(!trusted_link("javascript:alert(1)"));
        assert!(!trusted_link("https://soundcloud.com/a b"));
        assert!(!trusted_link("https://soundcloud.com\\@evil.example/"));
        assert!(cover_cache_allowed("https://i.ytimg.com/vi/abc/hqdefault.jpg"));
        assert!(!cover_cache_allowed("https://evil.example/pixel.png"));
    }

    #[test]
    fn sanitize_keeps_good_data_and_drops_the_rest() {
        let mut t = track();
        t.sanitize();
        assert_eq!(t.isrc.as_deref(), Some("GBDUW1300001"));
        assert!(t.cover_url.is_some() && t.permalink.is_some());

        let mut t = track();
        t.cover_url = Some("https://evil.example/pixel.png".into());
        t.permalink = Some("https://evil.example/login".into());
        t.isrc = Some("not an isrc".into());
        t.title = format!("A\u{0}B{}", "x".repeat(400));
        t.duration_ms = Some(0);
        t.external_ids.insert(ProviderId::Audius, "../x".into());
        t.external_ids.insert(ProviderId::YouTube, "7wtfhZwyrcc".into());
        t.external_ids.insert(ProviderId::Deezer, "999".into());
        t.external_ids.insert(ProviderId::LastFm, "Artist - Song".into());
        t.sanitize();
        assert_eq!((t.cover_url, t.permalink, t.isrc, t.duration_ms), (None, None, None, None));
        assert!(t.title.starts_with("AB") && t.title.chars().count() == MAX_TEXT_CHARS);
        assert_eq!(t.external_ids.len(), 2, "{:?}", t.external_ids);
        assert!(t.external_ids.contains_key(&ProviderId::YouTube) && t.external_ids.contains_key(&ProviderId::LastFm));
    }

    #[test]
    fn jam_requests_are_sanitized() {
        let mut t = track();
        t.permalink = Some("https://evil.example/login".into());
        let mut ops = crate::jam::OpsResponse {
            ops: vec![crate::jam::OpFrom { from: "friend".into(), op: crate::jam::Op::Add { track: t }, avatar: None }],
        };
        ops.sanitize();
        let crate::jam::Op::Add { track } = &ops.ops[0].op else { panic!() };
        assert_eq!(track.permalink, None);
    }

    #[test]
    fn claimed_ids() {
        let mut t = track();
        t.external_ids.insert(ProviderId::Audius, "au1".into());
        assert_eq!(t.claimed_id(ProviderId::Deezer), Some("123"));
        assert_eq!(t.claimed_id(ProviderId::Audius), Some("au1"));
        assert_eq!(t.claimed_id(ProviderId::YouTube), None);
        assert!(t.names_id(ProviderId::Audius, "au1") && !t.names_id(ProviderId::Audius, "au2"));
        t.id = "x/y".into();
        assert_eq!(t.claimed_id(ProviderId::Deezer), None);
    }
}
