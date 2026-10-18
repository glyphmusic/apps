//! Artist info for the artist page: a short bio and a photo from Wikipedia
//! (official MediaWiki / REST APIs, text under CC BY-SA, so the UI links the
//! article). Only pages whose short description says "musician" (in any of
//! the UI languages) are accepted, so a common name does not pull in a
//! footballer or a city.

use crate::paths;
use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct ArtistInfo {
    /// Plain-text summary (first paragraph).
    pub about: String,
    /// Article URL, shown as the source link.
    pub url: String,
    /// Cached photo, if the article has one.
    pub image: Option<PathBuf>,
}

/// Words in Wikidata short descriptions that mark a musical act.
const MUSIC_WORDS: &[&str] = &[
    "musician", "singer", "rapper", "band", "duo", "trio", "group", "dj", "producer", "songwriter", "composer",
    "vocalist", "guitarist", "drummer", "pianist", "record", "hip hop", "music", "musiker", "sänger", "sängerin",
    "komponist", "produzent", "рэп", "репер", "певец", "певица", "музыкант", "группа", "исполнитель", "продюсер",
    "композитор", "диджей", "автор песен",
];

fn is_musical(desc: &str) -> bool {
    let d = desc.to_lowercase();
    MUSIC_WORDS.iter().any(|w| d.contains(w))
}

fn names_match(name: &str, title: &str) -> bool {
    let n = crate::dedupe::normalize_artist(name);
    // "Muse (band)" -> "muse"
    let t = crate::dedupe::normalize_artist(title.split(" (").next().unwrap_or(title));
    !n.is_empty() && n == t
}

/// Looks the artist up on Wikipedia in `lang`, then English.
pub async fn lookup(http: &reqwest::Client, name: &str, lang: &str) -> Result<Option<ArtistInfo>> {
    lookup_via(http, name, lang, None).await
}

pub async fn lookup_via(http: &reqwest::Client, name: &str, lang: &str, server: Option<&crate::server_client::ServerClient>) -> Result<Option<ArtistInfo>> {
    let lang = if matches!(lang, "en" | "de" | "ru") { lang } else { "en" };
    let mut langs = vec![lang.to_string()];
    if lang != "en" {
        langs.push("en".into());
    }
    for l in langs {
        match lookup_in(http, name, &l, server).await {
            Ok(Some(info)) => return Ok(Some(info)),
            Ok(None) => {}
            Err(e) => tracing::debug!(lang = %l, "wikipedia lookup failed: {e}"),
        }
    }
    Ok(None)
}

async fn lookup_in(http: &reqwest::Client, name: &str, lang: &str, server: Option<&crate::server_client::ServerClient>) -> Result<Option<ArtistInfo>> {
    let api = format!("https://{lang}.wikipedia.org/w/api.php");
    let api = reqwest::Url::parse_with_params(&api, &[("action", "query"), ("list", "search"), ("srsearch", name), ("srlimit", "5"), ("format", "json")])?;
    let api = server.map(|s| s.upstream_url(api.as_str())).unwrap_or_else(|| api.into());
    let request = server.map(|s| s.authorize_url(http.get(&api),&api)).unwrap_or_else(|| http.get(&api));
    let found: Value = request
        .timeout(Duration::from_secs(8))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let titles: Vec<String> = found["query"]["search"]
        .as_array()
        .map(|a| a.iter().filter_map(|h| h["title"].as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    for title in titles.into_iter().filter(|t| names_match(name, t)) {
        let mut url = reqwest::Url::parse(&format!("https://{lang}.wikipedia.org/api/rest_v1/page/summary/"))?;
        url.path_segments_mut().map_err(|_| anyhow::anyhow!("bad url"))?.pop_if_empty().push(&title.replace(' ', "_"));
        let url = server.map(|s| s.upstream_url(url.as_str())).unwrap_or_else(|| url.into());
        let request = server.map(|s| s.authorize_url(http.get(&url),&url)).unwrap_or_else(|| http.get(&url));
        let s: Value = request.timeout(Duration::from_secs(8)).send().await?.error_for_status()?.json().await?;
        if s["type"].as_str() == Some("disambiguation") {
            continue;
        }
        if !is_musical(s["description"].as_str().unwrap_or("")) {
            continue;
        }
        let about = s["extract"].as_str().unwrap_or("").trim().to_string();
        if about.is_empty() {
            continue;
        }
        let page = s["content_urls"]["desktop"]["page"].as_str().unwrap_or("").to_string();
        let img_url = s["originalimage"]["source"].as_str().or(s["thumbnail"]["source"].as_str()).map(str::to_string);
        let image = match img_url {
            Some(u) => {
                let u = server.map(|s| s.upstream_url(&u)).unwrap_or(u);
                fetch_image(http, &u, server).await.ok()
            },
            None => None,
        };
        return Ok(Some(ArtistInfo { about, url: page, image }));
    }
    Ok(None)
}

/// Downloads the photo into the cache (longest edge 1280 px, JPEG).
async fn fetch_image(http: &reqwest::Client, url: &str, server: Option<&crate::server_client::ServerClient>) -> Result<PathBuf> {
    let dir = paths::cache_dir().join("artists");
    let path = dir.join(format!("{}.webp", paths::hash_hex(url)));
    if path.is_file() {
        return Ok(path);
    }
    let request = server.map(|s| s.authorize_url(http.get(url),url)).unwrap_or_else(|| http.get(url));
    let bytes = request.timeout(Duration::from_secs(15)).send().await?.error_for_status()?.bytes().await?;
    let dest = path.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let img = image::load_from_memory(&bytes)?;
        let img = if img.width().max(img.height()) > 1280 { img.thumbnail(1280, 1280) } else { img };
        let out = crate::profile_images::encode_webp(&img,80.0)?;
        std::fs::create_dir_all(&dir)?;
        paths::write_atomic(&dest, &out)
    })
    .await??;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn musical_descriptions() {
        assert!(is_musical("British rock band"));
        assert!(is_musical("российский рэп-исполнитель"));
        assert!(is_musical("deutscher Rapper"));
        assert!(!is_musical("Argentine footballer"));
        assert!(!is_musical("city in Germany"));
    }

    #[test]
    fn title_matching() {
        assert!(names_match("Muse", "Muse (band)"));
        assert!(names_match("The Chemical Brothers", "The Chemical Brothers"));
        assert!(!names_match("Muse", "Museum of Modern Art"));
    }
}
