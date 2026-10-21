//! Deezer library of a public profile (no login: Deezer hands out no new
//! OAuth apps, and profiles are public): favourites, playlists, albums,
//! artists.

use super::*;
use crate::providers::deezer::{convert, DTrack, Deezer};
use anyhow::{anyhow, bail};
use futures::stream::StreamExt;

const API: &str = "https://api.deezer.com";

pub struct DeezerLibrary {
    dz: Arc<Deezer>,
    user: String,
}

/// The numeric user id from a profile link (`deezer.com/<lang>/profile/123`)
/// or the id itself.
pub fn user_id(input: &str) -> Result<String> {
    let input = input.trim();
    if !input.is_empty() && input.chars().all(|c| c.is_ascii_digit()) {
        return Ok(input.to_string());
    }
    match crate::import::parse_url(input) {
        Some(crate::import::RemoteSource::DeezerUser(id)) => Ok(id),
        _ => bail!("paste the link to your Deezer profile (deezer.com/profile/…) or its number"),
    }
}

impl DeezerLibrary {
    pub fn new(dz: Arc<Deezer>, user: String) -> Self {
        Self { dz, user }
    }

    /// Every `data` item of a paged list (`next`), at most `max_pages`.
    async fn items(&self, first: String, max_pages: usize) -> Result<Vec<serde_json::Value>> {
        let mut out = Vec::new();
        let mut url = first;
        let mut query: Vec<(&str, &str)> = vec![("limit", "100")];
        for _ in 0..max_pages {
            let v = self.dz.get(&url, &query).await?;
            if let Some(items) = v["data"].as_array() {
                out.extend(items.iter().cloned());
            }
            match v["next"].as_str() {
                Some(n) => {
                    url = n.to_string();
                    query.clear();
                }
                None => break,
            }
        }
        Ok(out)
    }
}

fn total(v: &serde_json::Value) -> Option<usize> {
    v["total"].as_u64().map(|n| n as usize)
}

#[async_trait]
impl LibrarySource for DeezerLibrary {
    fn service(&self) -> Service {
        Service::Deezer
    }

    async fn overview(&self) -> Result<Overview> {
        let u = &self.user;
        let me = self.dz.get(&format!("{API}/user/{u}"), &[]).await.map_err(|_| anyhow!("no public Deezer profile with this number"))?;
        let account = me["name"].as_str().unwrap_or("Deezer").to_string();
        let one = [("limit", "1")];
        let liked = self.dz.get(&format!("{API}/user/{u}/tracks"), &one).await?;
        let albums = self.dz.get(&format!("{API}/user/{u}/albums"), &one).await?;
        let artists = self.dz.get(&format!("{API}/user/{u}/artists"), &one).await?;
        let lists = self.items(format!("{API}/user/{u}/playlists"), 20).await?;
        let mut collections = vec![Collection::liked(total(&liked)), Collection::albums(total(&albums)), Collection::artists(total(&artists))];
        for p in &lists {
            // "Loved tracks" is the favourites list again.
            if p["is_loved_track"].as_bool() == Some(true) {
                continue;
            }
            let Some(id) = p["id"].as_u64().map(|n| n.to_string()).or_else(|| p["id"].as_str().map(str::to_string)) else { continue };
            let name = p["title"].as_str().unwrap_or("Deezer playlist");
            let count = p["nb_tracks"].as_u64().map(|n| n as usize);
            let cover = p["picture_xl"].as_str().or(p["picture_big"].as_str()).map(str::to_string);
            let mut c = Collection::playlist(&id, name, count, cover, Some(format!("https://www.deezer.com/playlist/{id}")));
            c.followed = p["creator"]["id"].as_u64().map(|n| n.to_string()).is_some_and(|c| &c != u);
            collections.push(c);
        }
        Ok(Overview { account, collections, notes: vec!["deezer_public".into()] })
    }

    async fn fetch(&self, c: &Collection) -> Result<Fetched> {
        let u = &self.user;
        match c.kind {
            Kind::Liked => {
                let items = self.items(format!("{API}/user/{u}/tracks"), 100).await?;
                Ok(Fetched::Tracks(items.into_iter().filter_map(|v| serde_json::from_value::<DTrack>(v).ok()).filter_map(convert).collect()))
            }
            Kind::Playlist => Ok(Fetched::Tracks(self.dz.playlist(c.playlist_id()).await?.1)),
            Kind::Albums => {
                let items = self.items(format!("{API}/user/{u}/albums"), 30).await?;
                let dz = self.dz.clone();
                let albums = futures::stream::iter(items.into_iter().filter_map(|a| {
                    let id = a["id"].as_u64().map(|n| n.to_string())?;
                    Some((id, a))
                }))
                .map(|(id, a)| {
                    let dz = dz.clone();
                    async move {
                        let release = crate::providers::deezer::Release {
                            id,
                            title: a["title"].as_str().unwrap_or("").to_string(),
                            artist: a["artist"]["name"].as_str().unwrap_or("").to_string(),
                            cover_url: a["cover_xl"].as_str().or(a["cover_big"].as_str()).map(str::to_string),
                            year: None,
                            released: None,
                            kind: crate::catalog::AlbumKind::Album,
                        };
                        let tracks = dz.album_tracks(&release).await.unwrap_or_default();
                        SavedAlbum { title: release.title, artist: release.artist, tracks }
                    }
                })
                .buffered(3)
                .filter(|a| futures::future::ready(!a.title.is_empty()))
                .collect::<Vec<_>>()
                .await;
                Ok(Fetched::Albums(albums))
            }
            Kind::Artists => {
                let items = self.items(format!("{API}/user/{u}/artists"), 30).await?;
                Ok(Fetched::Artists(items.iter().filter_map(|a| a["name"].as_str()).map(str::to_string).collect()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_input() {
        assert_eq!(user_id(" 12345 ").unwrap(), "12345");
        assert_eq!(user_id("https://www.deezer.com/de/profile/987").unwrap(), "987");
        assert!(user_id("someone").is_err());
        assert!(user_id("https://www.deezer.com/playlist/1").is_err());
    }
}
