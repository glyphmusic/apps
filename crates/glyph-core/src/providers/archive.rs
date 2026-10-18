//! Internet Archive: Advanced Search API, native playback, restricted to
//! audio items in freely licensed collections (Creative Commons / public domain).

use crate::config::AudioQuality;
use crate::provider::*;
use anyhow::Result;
use async_trait::async_trait;
use glyph_protocol::{Capabilities, PlaybackSource, ProviderId, Track};
use serde::Deserialize;

pub struct Archive {
    http: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct SearchResp {
    response: SearchBody,
}

#[derive(Debug, Deserialize)]
struct SearchBody {
    #[serde(default)]
    docs: Vec<Doc>,
}

#[derive(Debug, Deserialize)]
struct Doc {
    identifier: String,
    #[serde(default, deserialize_with = "de_flex_first_string")]
    title: Option<String>,
    #[serde(default, deserialize_with = "de_flex_first_string")]
    creator: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MetaResp {
    #[serde(default)]
    files: Vec<MetaFile>,
    #[serde(default)]
    metadata: MetaInfo,
}

#[derive(Debug, Default, Deserialize)]
struct MetaInfo {
    #[serde(default, deserialize_with = "de_flex_first_string")]
    title: Option<String>,
    #[serde(default, deserialize_with = "de_flex_first_string")]
    creator: Option<String>,
    #[serde(default, deserialize_with = "de_flex_first_string")]
    licenseurl: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MetaFile {
    name: String,
    #[serde(default)]
    format: Option<String>,
    #[serde(default, deserialize_with = "de_opt_f64")]
    length: Option<f64>,
}

fn doc_to_track(d: Doc) -> Track {
    Track {
        cover_url: Some(format!("https://archive.org/services/img/{}", d.identifier)),
        permalink: Some(format!("https://archive.org/details/{}", d.identifier)),
        title: d.title.unwrap_or_else(|| d.identifier.clone()),
        artist: d.creator.unwrap_or_else(|| "Unknown".into()),
        id: d.identifier,
        source: ProviderId::Archive,
        album: None,
        duration_ms: None,
        isrc: None,
        external_ids: Default::default(),
        explicit: false,
    }
}

/// Free-licence check on an item's `licenseurl`.
pub fn is_free_license(url: &str) -> bool {
    let u = url.to_lowercase();
    u.contains("creativecommons.org") || u.contains("publicdomain")
}

/// Strips Lucene operators from user input.
fn lucene_terms(q: &str) -> Vec<String> {
    q.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .filter(|w| !matches!(*w, "AND" | "OR" | "NOT" | "TO"))
        .map(str::to_string)
        .collect()
}

/// Builds the advanced-search query (audio only, free collections + free licence).
pub fn build_query(q: &str) -> Option<String> {
    let terms = lucene_terms(q);
    if terms.is_empty() {
        return None;
    }
    let all = terms.join(" AND ");
    Some(format!(
        "(title:({all}) OR creator:({all})) AND mediatype:audio \
         AND collection:(opensource_audio OR netlabels) \
         AND (licenseurl:*creativecommons* OR licenseurl:*publicdomain*)"
    ))
}

/// Picks the best natively decodable file (MP3 first, then Ogg Vorbis).
fn pick_file(files: &[MetaFile]) -> Option<&MetaFile> {
    let rank = |f: &MetaFile| -> Option<u8> {
        let fmt = f.format.as_deref()?.to_lowercase();
        if fmt.contains("vbr mp3") {
            Some(0)
        } else if fmt.contains("mp3") {
            Some(1)
        } else if fmt.contains("ogg vorbis") {
            Some(2)
        } else {
            None
        }
    };
    files.iter().filter_map(|f| rank(f).map(|r| (r, f))).min_by_key(|(r, _)| *r).map(|(_, f)| f)
}

/// The download file for `q`: the played file's FLAC original (High) or its
/// 64 kbps MP3 derivative (Low), else the played file itself. Siblings share
/// the name stem ("01 Song.flac", "01 Song.mp3", "01 Song_64kb.mp3").
fn pick_download_file(files: &[MetaFile], q: AudioQuality) -> Option<&MetaFile> {
    let played = pick_file(files)?;
    let stem = |n: &str| -> String {
        let base = n.rsplit_once('.').map_or(n, |(b, _)| b);
        base.trim_end_matches("_64kb").trim_end_matches("_vbr").to_lowercase()
    };
    let want = stem(&played.name);
    let fmt_has = |f: &MetaFile, s: &str| f.format.as_deref().is_some_and(|x| x.to_lowercase().contains(s));
    let sibling = |s: &str| files.iter().find(|f| fmt_has(f, s) && stem(&f.name) == want);
    let better = match q.effective() {
        AudioQuality::High => sibling("flac"),
        AudioQuality::Low => sibling("64kbps"),
        AudioQuality::Auto | AudioQuality::Standard => None,
    };
    Some(better.unwrap_or(played))
}

fn encode_path(name: &str) -> String {
    name.split('/')
        .map(|seg| {
            seg.bytes()
                .map(|b| match b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
                    _ => format!("%{b:02X}"),
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}

impl Archive {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }

    async fn metadata(&self, id: &str) -> Result<MetaResp> {
        let m: MetaResp = send_json(self.http.get(format!("https://archive.org/metadata/{id}"))).await?;
        let free = m.metadata.licenseurl.as_deref().map(is_free_license).unwrap_or(false);
        if !free {
            // Never play items without a free licence.
            return Err(ProviderError::NotFound.into());
        }
        Ok(m)
    }
}

#[async_trait]
impl MusicProvider for Archive {
    fn id(&self) -> ProviderId {
        ProviderId::Archive
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::FullPlayback
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        let Some(q) = build_query(query) else { return Ok(vec![]) };
        let req = self.http.get("https://archive.org/advancedsearch.php").query(&[
            ("q", q.as_str()),
            ("fl[]", "identifier"),
            ("fl[]", "title"),
            ("fl[]", "creator"),
            ("sort[]", "downloads desc"),
            ("rows", "15"),
            ("output", "json"),
        ]);
        let r: SearchResp = send_json(req).await?;
        Ok(r.response.docs.into_iter().map(doc_to_track).collect())
    }

    async fn track(&self, id: &str) -> Result<Track> {
        let m = self.metadata(id).await?;
        let mut t = doc_to_track(Doc {
            identifier: id.to_string(),
            title: m.metadata.title.clone(),
            creator: m.metadata.creator.clone(),
        });
        t.duration_ms = pick_file(&m.files).and_then(|f| f.length).and_then(secs_to_ms);
        Ok(t)
    }

    async fn playback_source(&self, track: &Track) -> Result<PlaybackSource> {
        let id = id_at(ProviderId::Archive, track).ok_or(ProviderError::Unsupported)?;
        let m = self.metadata(id).await?;
        let file = pick_file(&m.files).ok_or(ProviderError::NotFound)?;
        let mime = if file.format.as_deref().map(|f| f.to_lowercase().contains("ogg")).unwrap_or(false) {
            "audio/ogg"
        } else {
            "audio/mpeg"
        };
        Ok(PlaybackSource::Stream {
            provider: ProviderId::Archive,
            url: format!("https://archive.org/download/{id}/{}", encode_path(&file.name)),
            mime: Some(mime.into()),
        })
    }

    /// Search is limited to Creative-Commons / public-domain items, which may
    /// be saved: the streamed file, or its FLAC / 64 kbps sibling depending
    /// on the download quality.
    async fn download_url(&self, track: &Track) -> Result<Option<String>> {
        let Some(id) = id_at(ProviderId::Archive, track) else { return Ok(None) };
        let m = self.metadata(id).await?;
        let file = pick_download_file(&m.files, crate::downloads::job_quality()).ok_or(ProviderError::NotFound)?;
        Ok(Some(format!("https://archive.org/download/{id}/{}", encode_path(&file.name))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_is_sanitised_and_restricted() {
        let q = build_query("jazz: \"blue\" (OR) *").unwrap();
        assert!(q.contains("title:(jazz AND blue)"));
        assert!(q.contains("mediatype:audio"));
        assert!(q.contains("licenseurl:*creativecommons*"));
        assert!(!q.contains('"'));
        assert!(build_query("*** ()").is_none());
    }

    #[test]
    fn parses_docs_with_array_fields() {
        let json = r#"{"response":{"numFound":2,"docs":[
          {"identifier":"abc","title":"T","creator":["A","B"]},
          {"identifier":"def"}]}}"#;
        let r: SearchResp = serde_json::from_str(json).unwrap();
        let tracks: Vec<Track> = r.response.docs.into_iter().map(doc_to_track).collect();
        assert_eq!(tracks[0].artist, "A");
        assert_eq!(tracks[1].title, "def");
        assert_eq!(tracks[1].artist, "Unknown");
    }

    #[test]
    fn download_quality_picks_siblings() {
        let f = |name: &str, format: &str| MetaFile { name: name.into(), format: Some(format.into()), length: Some(10.0) };
        let files = vec![
            f("02 Other.flac", "Flac"),
            f("01 Song.flac", "Flac"),
            f("01 Song.mp3", "VBR MP3"),
            f("01 Song_64kb.mp3", "64Kbps MP3"),
            f("02 Other.mp3", "VBR MP3"),
        ];
        assert_eq!(pick_download_file(&files, AudioQuality::Standard).unwrap().name, "01 Song.mp3");
        assert_eq!(pick_download_file(&files, AudioQuality::High).unwrap().name, "01 Song.flac");
        assert_eq!(pick_download_file(&files, AudioQuality::Low).unwrap().name, "01 Song_64kb.mp3");
    }

    #[test]
    fn picks_mp3_and_encodes_path() {
        let files = vec![
            MetaFile { name: "a.flac".into(), format: Some("Flac".into()), length: None },
            MetaFile { name: "b c.mp3".into(), format: Some("128Kbps MP3".into()), length: Some(10.0) },
            MetaFile { name: "d.mp3".into(), format: Some("VBR MP3".into()), length: Some(10.0) },
        ];
        assert_eq!(pick_file(&files).unwrap().name, "d.mp3");
        // No FLAC sibling of the played file: stays on it.
        assert_eq!(pick_download_file(&files, AudioQuality::High).unwrap().name, "d.mp3");
        assert_eq!(encode_path("sub dir/b c.mp3"), "sub%20dir/b%20c.mp3");
        assert!(is_free_license("https://creativecommons.org/licenses/by/4.0/"));
        assert!(!is_free_license("http://example.com/all-rights"));
    }
}
