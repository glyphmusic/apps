//! Shared wire types and canonical genre/mood tag normalization for Glyph's
//! hybrid music taste engine (local privacy-first profile + optional
//! server-side collaborative filtering and tag cache).

use crate::Track;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Batch request for normalized genre/mood/similar tags (`POST /v1/taste/tags`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TasteTagsRequest {
    #[serde(default)]
    pub artists: Vec<String>,
}

/// Enriched metadata for one artist, cached globally on the server.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArtistTags {
    pub artist: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub moods: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub similar: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TasteTagsResponse {
    #[serde(default)]
    pub items: Vec<ArtistTags>,
}

/// Anonymized artist affinity sent to `/v1/taste/collab`.
/// Never contains user handles, timestamps, or raw playback history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TasteArtistSignal {
    pub artist: String,
    /// Normalized preference strength in `0.0..=1.0`.
    pub weight: f32,
}

/// Request for collaborative artist recommendations (`POST /v1/taste/collab`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TasteCollabRequest {
    /// User's top liked/listened artists with normalized weights.
    #[serde(default)]
    pub positive: Vec<TasteArtistSignal>,
    /// Artists to exclude from recommendations (hidden, disliked, high-skip).
    #[serde(default)]
    pub negative: Vec<String>,
    /// Genre/mood tags the user avoids (penalized/filtered).
    #[serde(default)]
    pub avoided_tags: Vec<String>,
    /// Max recommended artists to return (default 15, max 30).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Whether the server may incorporate this anonymized profile into the
    /// community co-occurrence graph.
    #[serde(default = "default_true")]
    pub contribute: bool,
}

fn default_true() -> bool {
    true
}

/// One artist recommended by collaborative filtering / community co-occurrence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollabArtistRec {
    pub artist: String,
    /// Recommendation strength (`> 0.0`).
    pub score: f32,
    /// Up to 3 seed artists from the user's profile that drove this match.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub because_of: Vec<String>,
    /// Genres/tags associated with this recommended artist.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    /// Whether this recommendation came from Glyph community co-occurrence
    /// (`true`) or metadata/related-artist cold-start fallback (`false`).
    #[serde(default)]
    pub collaborative: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TasteCollabResponse {
    #[serde(default)]
    pub recommendations: Vec<CollabArtistRec>,
    /// Approximate number of unique community contributors reflected in matched seeds.
    #[serde(default)]
    pub contributors: u32,
    /// True when collaborative co-occurrence data contributed directly.
    #[serde(default)]
    pub model_ready: bool,
}

/// One starter artist offered during onboarding (`GET /v1/taste/onboarding`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OnboardingArtist {
    pub name: String,
    pub genre: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub photo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_track: Option<Track>,
}

/// Starter artists and songs offered in the onboarding taste picker.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OnboardingCatalogResponse {
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub artists: Vec<OnboardingArtist>,
    #[serde(default)]
    pub tracks: Vec<Track>,
}

// ------------------------------------------------ canonical tag normalization

/// Noise tags frequently returned by community taggers (Last.fm, MusicBrainz)
/// that describe listener bookkeeping rather than musical sound.
const NOISE_TAGS: &[&str] = &[
    "seen live",
    "favorites",
    "favourites",
    "favorite",
    "favourite",
    "loved",
    "awesome",
    "good",
    "great",
    "best",
    "spotify",
    "albums i own",
    "my music",
    "under 2000 listeners",
    "all",
    "music",
    "male vocalists",
    "female vocalists",
    "singer",
    "band",
    "american",
    "british",
    "uk",
    "usa",
    "german",
    "french",
    "swedish",
    "canadian",
    "australian",
    "japanese",
    "korean",
    "russian",
    "polish",
    "spanish",
    "italian",
    "brazilian",
    "check out",
    "to listen",
    "fip",
    "radio",
    "2000s",
    "2010s",
    "2020s",
    "90s",
    "80s",
    "70s",
    "60s",
    "00s",
    "10s",
    "20s",
];

/// Canonical genre rules: `(needle, canonical_label)`.
/// Checked longest/most-specific first.
const GENRE_RULES: &[(&str, &str)] = &[
    ("drum and bass", "Drum & Bass"),
    ("drum & bass", "Drum & Bass"),
    ("dnb", "Drum & Bass"),
    ("liquid funk", "Drum & Bass"),
    ("psychedelic rock", "Psychedelic"),
    ("psychedelic pop", "Psychedelic"),
    ("neo-psychedelia", "Psychedelic"),
    ("psychedelic", "Psychedelic"),
    ("dream pop", "Dream Pop"),
    ("dreampop", "Dream Pop"),
    ("shoegaze", "Shoegaze"),
    ("synthpop", "Synthpop"),
    ("synth-pop", "Synthpop"),
    ("synth pop", "Synthpop"),
    ("synthwave", "Synthwave"),
    ("retrowave", "Synthwave"),
    ("darkwave", "Darkwave"),
    ("chillwave", "Chillwave"),
    ("vaporwave", "Vaporwave"),
    ("post-punk", "Post-Punk"),
    ("post punk", "Post-Punk"),
    ("post-rock", "Post-Rock"),
    ("post rock", "Post-Rock"),
    ("indie rock", "Indie Rock"),
    ("indie pop", "Indie Pop"),
    ("indie folk", "Indie Folk"),
    ("bedroom pop", "Indie Pop"),
    ("alternative rock", "Alternative"),
    ("alt-rock", "Alternative"),
    ("alt rock", "Alternative"),
    ("alternative r&b", "R&B"),
    ("alt r&b", "R&B"),
    ("alternative", "Alternative"),
    ("indie", "Indie"),
    ("hip-hop", "Hip-Hop"),
    ("hip hop", "Hip-Hop"),
    ("hiphop", "Hip-Hop"),
    ("boom bap", "Hip-Hop"),
    ("deutschrap", "Rap"),
    ("german rap", "Rap"),
    ("cloud rap", "Trap & Cloud Rap"),
    ("trap", "Trap"),
    ("drill", "Drill"),
    ("rap", "Rap"),
    ("r&b", "R&B"),
    ("rnb", "R&B"),
    ("rhythm and blues", "R&B"),
    ("neo-soul", "Soul"),
    ("neo soul", "Soul"),
    ("soul", "Soul"),
    ("funk", "Funk"),
    ("disco", "Disco"),
    ("nu-disco", "Disco"),
    ("deep house", "House"),
    ("tech house", "House"),
    ("french house", "House"),
    ("progressive house", "House"),
    ("house", "House"),
    ("techno", "Techno"),
    ("trance", "Trance"),
    ("dubstep", "Dubstep"),
    ("uk garage", "UK Garage"),
    ("garage", "UK Garage"),
    ("idm", "IDM"),
    ("downtempo", "Downtempo"),
    ("trip-hop", "Trip-Hop"),
    ("trip hop", "Trip-Hop"),
    ("ambient", "Ambient"),
    ("lo-fi", "Lo-Fi"),
    ("lofi", "Lo-Fi"),
    ("chillhop", "Lo-Fi"),
    ("edm", "Electronic"),
    ("electropop", "Electropop"),
    ("electro", "Electronic"),
    ("electronic", "Electronic"),
    ("electronica", "Electronic"),
    ("hyperpop", "Hyperpop"),
    ("k-pop", "K-Pop"),
    ("kpop", "K-Pop"),
    ("j-pop", "J-Pop"),
    ("jpop", "J-Pop"),
    ("city pop", "City Pop"),
    ("pop rock", "Pop"),
    ("art pop", "Art Pop"),
    ("pop", "Pop"),
    ("heavy metal", "Metal"),
    ("death metal", "Metal"),
    ("black metal", "Metal"),
    ("thrash metal", "Metal"),
    ("nu metal", "Metal"),
    ("nu-metal", "Metal"),
    ("metalcore", "Metalcore"),
    ("progressive metal", "Metal"),
    ("metal", "Metal"),
    ("hardcore punk", "Punk"),
    ("pop punk", "Punk"),
    ("pop-punk", "Punk"),
    ("punk rock", "Punk"),
    ("punk", "Punk"),
    ("emo", "Emo"),
    ("grunge", "Grunge"),
    ("hard rock", "Rock"),
    ("classic rock", "Rock"),
    ("progressive rock", "Rock"),
    ("prog rock", "Rock"),
    ("blues rock", "Rock"),
    ("garage rock", "Rock"),
    ("rock", "Rock"),
    ("jazz fusion", "Jazz"),
    ("nu jazz", "Jazz"),
    ("smooth jazz", "Jazz"),
    ("jazz", "Jazz"),
    ("blues", "Blues"),
    ("classical", "Classical"),
    ("neoclassical", "Classical"),
    ("orchestral", "Classical"),
    ("soundtrack", "Soundtrack"),
    ("ost", "Soundtrack"),
    ("film score", "Soundtrack"),
    ("piano", "Instrumental"),
    ("instrumental", "Instrumental"),
    ("singer-songwriter", "Folk & Acoustic"),
    ("singer songwriter", "Folk & Acoustic"),
    ("acoustic", "Folk & Acoustic"),
    ("folk", "Folk & Acoustic"),
    ("country", "Country"),
    ("americana", "Country"),
    ("reggae", "Reggae"),
    ("dub", "Reggae"),
    ("reggaeton", "Latin"),
    ("latin", "Latin"),
    ("bossa nova", "Bossa Nova"),
    ("afrobeats", "Afrobeats"),
    ("afrobeat", "Afrobeats"),
    ("afropop", "Afrobeats"),
    ("dancehall", "Dancehall"),
    ("dance", "Dance"),
    ("phonk", "Phonk"),
    ("gospel", "Soul"),
    ("new wave", "New Wave"),
    ("britpop", "Indie Rock"),
];

/// Mood rules mapping raw tags or canonical genres to user-facing moods.
const MOOD_TAG_RULES: &[(&str, &str)] = &[
    ("chill", "Chill"),
    ("chillout", "Chill"),
    ("relax", "Chill"),
    ("mellow", "Chill"),
    ("calm", "Chill"),
    ("laid-back", "Chill"),
    ("melanchol", "Melancholic"),
    ("sad", "Melancholic"),
    ("bittersweet", "Melancholic"),
    ("depressive", "Melancholic"),
    ("heartbreak", "Melancholic"),
    ("dreamy", "Dreamy"),
    ("ethereal", "Dreamy"),
    ("hazy", "Dreamy"),
    ("energetic", "Energetic"),
    ("upbeat", "Upbeat"),
    ("happy", "Upbeat"),
    ("feel good", "Upbeat"),
    ("summer", "Upbeat"),
    ("party", "Energetic"),
    ("workout", "Energetic"),
    ("hype", "Energetic"),
    ("aggressive", "Intense"),
    ("heavy", "Intense"),
    ("angry", "Intense"),
    ("dark", "Dark"),
    ("gothic", "Dark"),
    ("moody", "Dark"),
    ("noir", "Dark"),
    ("atmospheric", "Atmospheric"),
    ("cinematic", "Atmospheric"),
    ("space", "Atmospheric"),
    ("ambient", "Atmospheric"),
    ("focus", "Focus"),
    ("study", "Focus"),
    ("coding", "Focus"),
    ("work", "Focus"),
    ("night", "Late Night"),
    ("nocturnal", "Late Night"),
    ("midnight", "Late Night"),
    ("groovy", "Groovy"),
    ("funky", "Groovy"),
    ("danceable", "Groovy"),
    ("nostalgic", "Nostalgic"),
    ("retro", "Nostalgic"),
    ("vintage", "Nostalgic"),
    ("warm", "Warm"),
    ("cozy", "Warm"),
    ("intimate", "Warm"),
    ("romantic", "Warm"),
];

/// Default mood associations inferred from canonical genres when explicit mood
/// tags are absent.
const GENRE_MOODS: &[(&str, &[&str])] = &[
    ("Dream Pop", &["Dreamy", "Atmospheric"]),
    ("Shoegaze", &["Dreamy", "Atmospheric"]),
    ("Psychedelic", &["Dreamy", "Groovy"]),
    ("Synthwave", &["Late Night", "Nostalgic"]),
    ("Darkwave", &["Dark", "Late Night"]),
    ("Chillwave", &["Chill", "Dreamy"]),
    ("Lo-Fi", &["Chill", "Focus"]),
    ("Ambient", &["Atmospheric", "Focus"]),
    ("Downtempo", &["Chill", "Late Night"]),
    ("Trip-Hop", &["Dark", "Late Night"]),
    ("House", &["Groovy", "Energetic"]),
    ("Techno", &["Energetic", "Dark"]),
    ("Drum & Bass", &["Energetic", "Intense"]),
    ("Electronic", &["Energetic", "Atmospheric"]),
    ("Synthpop", &["Upbeat", "Nostalgic"]),
    ("Electropop", &["Upbeat", "Energetic"]),
    ("Hyperpop", &["Energetic", "Intense"]),
    ("Pop", &["Upbeat"]),
    ("K-Pop", &["Upbeat", "Energetic"]),
    ("City Pop", &["Nostalgic", "Groovy"]),
    ("Hip-Hop", &["Groovy", "Energetic"]),
    ("Rap", &["Energetic", "Intense"]),
    ("Trap", &["Dark", "Energetic"]),
    ("Trap & Cloud Rap", &["Late Night", "Atmospheric"]),
    ("Drill", &["Dark", "Intense"]),
    ("Phonk", &["Dark", "Energetic"]),
    ("R&B", &["Warm", "Late Night"]),
    ("Soul", &["Warm", "Groovy"]),
    ("Funk", &["Groovy", "Upbeat"]),
    ("Disco", &["Groovy", "Upbeat"]),
    ("Indie", &["Chill", "Nostalgic"]),
    ("Indie Rock", &["Nostalgic", "Energetic"]),
    ("Indie Pop", &["Dreamy", "Upbeat"]),
    ("Indie Folk", &["Warm", "Melancholic"]),
    ("Folk & Acoustic", &["Warm", "Chill"]),
    ("Alternative", &["Melancholic", "Energetic"]),
    ("Post-Punk", &["Dark", "Melancholic"]),
    ("Post-Rock", &["Atmospheric", "Melancholic"]),
    ("Rock", &["Energetic"]),
    ("Punk", &["Intense", "Energetic"]),
    ("Emo", &["Melancholic", "Intense"]),
    ("Grunge", &["Dark", "Intense"]),
    ("Metal", &["Intense", "Dark"]),
    ("Metalcore", &["Intense"]),
    ("Jazz", &["Chill", "Warm"]),
    ("Blues", &["Warm", "Melancholic"]),
    ("Bossa Nova", &["Chill", "Warm"]),
    ("Classical", &["Focus", "Atmospheric"]),
    ("Soundtrack", &["Atmospheric", "Focus"]),
    ("Instrumental", &["Focus", "Chill"]),
    ("Reggae", &["Chill", "Warm"]),
    ("Latin", &["Upbeat", "Groovy"]),
    ("Afrobeats", &["Groovy", "Upbeat"]),
    ("Dance", &["Energetic", "Upbeat"]),
    ("New Wave", &["Nostalgic", "Upbeat"]),
];

fn clean_tag(raw: &str) -> String {
    raw.trim()
        .to_lowercase()
        .replace(['_', '\t', '\n', '\r'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_noise(tag: &str, artist_lower: &str) -> bool {
    if tag.len() < 2 || tag.len() > 40 {
        return true;
    }
    if tag.chars().all(|c| c.is_ascii_digit() || c == 's' || c == '-') {
        return true;
    }
    if !artist_lower.is_empty() {
        let t_bare = tag.strip_prefix("the ").unwrap_or(tag);
        let a_bare = artist_lower.strip_prefix("the ").unwrap_or(artist_lower);
        if tag == artist_lower || (!t_bare.is_empty() && t_bare == a_bare) {
            return true;
        }
    }
    NOISE_TAGS.contains(&tag)
}

fn match_genre(tag: &str) -> Option<&'static str> {
    for &(needle, canonical) in GENRE_RULES {
        if tag == needle {
            return Some(canonical);
        }
    }
    // Word-level containment fallback for compound tags like "french indie pop" or "uk rap".
    let padded = format!(" {tag} ");
    let dehyphen = format!(" {} ", tag.replace(['-', '/'], " "));
    for &(needle, canonical) in GENRE_RULES {
        let token = format!(" {needle} ");
        if padded.contains(&token) || dehyphen.contains(&token) {
            return Some(canonical);
        }
    }
    None
}

fn match_mood(tag: &str) -> Option<&'static str> {
    let padded = format!(" {} ", tag.replace(['-', '/'], " "));
    let words: Vec<&str> = padded.split_whitespace().collect();
    for &(needle, mood) in MOOD_TAG_RULES {
        if tag == needle || padded.contains(&format!(" {needle} ")) {
            return Some(mood);
        }
        if needle.len() >= 5 && words.iter().any(|w| w.starts_with(needle)) {
            return Some(mood);
        }
    }
    None
}

/// Normalizes a list of raw provider tags (e.g. from Last.fm, Deezer, or
/// MusicBrainz) into deduplicated `(genres, moods, clean_tags)`.
pub fn classify_tags(raw_tags: &[String], artist_name: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let artist_lower = clean_tag(artist_name);
    let mut genres: Vec<String> = Vec::new();
    let mut moods: Vec<String> = Vec::new();
    let mut clean: Vec<String> = Vec::new();
    let mut seen_g: HashSet<String> = HashSet::new();
    let mut seen_m: HashSet<String> = HashSet::new();
    let mut seen_c: HashSet<String> = HashSet::new();

    for raw in raw_tags {
        let tag = clean_tag(raw);
        if is_noise(&tag, &artist_lower) {
            continue;
        }
        if let Some(g) = match_genre(&tag) {
            if genres.len() < 6 && seen_g.insert(g.to_string()) {
                genres.push(g.to_string());
            }
        }
        if let Some(m) = match_mood(&tag) {
            if moods.len() < 5 && seen_m.insert(m.to_string()) {
                moods.push(m.to_string());
            }
        }
        if clean.len() < 10 && seen_c.insert(tag.clone()) {
            clean.push(tag);
        }
    }

    // Fill moods from canonical genres if provider returned genres without mood words.
    if moods.len() < 3 {
        for g in &genres {
            if let Some((_, defaults)) = GENRE_MOODS.iter().find(|(name, _)| *name == g.as_str()) {
                for &m in *defaults {
                    if moods.len() >= 4 {
                        break;
                    }
                    if seen_m.insert(m.to_string()) {
                        moods.push(m.to_string());
                    }
                }
            }
        }
    }

    (genres, moods, clean)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_tags_extracts_genres_moods_and_drops_noise() {
        let raw: Vec<String> = [
            "seen live",
            "Tame Impala",
            "psychedelic rock",
            "dream pop",
            "2010s",
            "australian",
            "chill",
            "indie",
            "favorites",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let (genres, moods, clean) = classify_tags(&raw, "Tame Impala");
        assert!(genres.contains(&"Psychedelic".to_string()));
        assert!(genres.contains(&"Dream Pop".to_string()));
        assert!(genres.contains(&"Indie".to_string()));
        assert!(moods.contains(&"Chill".to_string()));
        assert!(moods.contains(&"Dreamy".to_string()));
        assert!(!clean.iter().any(|t| t == "seen live" || t == "favorites" || t == "tame impala" || t == "2010s"));
    }

    #[test]
    fn classify_tags_infers_moods_from_genres() {
        let raw = vec!["synthwave".to_string(), "darkwave".to_string()];
        let (genres, moods, _) = classify_tags(&raw, "Perturbator");
        assert_eq!(genres, vec!["Synthwave", "Darkwave"]);
        assert!(moods.contains(&"Late Night".to_string()));
        assert!(moods.contains(&"Dark".to_string()));
    }

    #[test]
    fn classify_tags_handles_empty_and_compound() {
        let (g, m, c) = classify_tags(&[], "Nobody");
        assert!(g.is_empty() && m.is_empty() && c.is_empty());

        let raw = vec!["french indie pop".to_string(), "drum and bass".to_string()];
        let (g, _, _) = classify_tags(&raw, "Artist");
        assert!(g.contains(&"Indie Pop".to_string()));
        assert!(g.contains(&"Drum & Bass".to_string()));
    }
}
