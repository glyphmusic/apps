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

    /// Every `data` item of a paged list, fetching pages in parallel (6 at a
    /// time) once page 0 reports `total`, preserving exact order.
    async fn items(&self, first: String, max_pages: usize) -> Result<Vec<serde_json::Value>> {
        self.items_with_progress(first, max_pages, None, None, None).await
    }

    async fn items_with_progress(
        &self,
        first: String,
        max_pages: usize,
        label: Option<&str>,
        progress: Option<&crate::flume_like::Sender<crate::ImportProgress>>,
        cancel: Option<&Arc<std::sync::atomic::AtomicBool>>,
    ) -> Result<Vec<serde_json::Value>> {
        use std::sync::atomic::Ordering;
        if max_pages == 0 {
            return Ok(Vec::new());
        }
        if cancel.is_some_and(|f| f.load(Ordering::Relaxed)) {
            bail!("cancelled");
        }
        let v = self.dz.get(&first, &[("limit", "100"), ("index", "0")]).await?;
        let mut out: Vec<serde_json::Value> = v["data"].as_array().cloned().unwrap_or_default();
        let known_total = total(&v).map(|t| t.min(max_pages * 100));
        if let (Some(tx), Some(lbl)) = (progress, label) {
            let tot = if v["next"].is_null() { out.len() } else { known_total.unwrap_or(0) };
            let _ = tx.send(crate::ImportProgress { label: lbl.to_string(), done: out.len(), total: tot });
        }
        if v["next"].is_null() || max_pages == 1 {
            return Ok(out);
        }
        if let Some(tot) = known_total.filter(|&t| t > 100) {
            let offsets: Vec<usize> = (100..tot).step_by(100).take(max_pages - 1).collect();
            let mut pages: Vec<Vec<serde_json::Value>> = vec![Vec::new(); offsets.len()];
            let mut done = out.len();
            let dz = self.dz.clone();
            let mut stream = futures::stream::iter(offsets.into_iter().enumerate())
                .map(|(slot, idx)| {
                    let dz = dz.clone();
                    let url = first.clone();
                    async move {
                        if cancel.is_some_and(|f| f.load(Ordering::Relaxed)) {
                            bail!("cancelled");
                        }
                        let off = idx.to_string();
                        let v = dz.get(&url, &[("limit", "100"), ("index", off.as_str())]).await?;
                        Ok::<_, anyhow::Error>((slot, v["data"].as_array().cloned().unwrap_or_default()))
                    }
                })
                .buffer_unordered(6);
            while let Some(res) = stream.next().await {
                if cancel.is_some_and(|f| f.load(Ordering::Relaxed)) {
                    bail!("cancelled");
                }
                let (slot, items) = res?;
                done += items.len();
                pages[slot] = items;
                if let (Some(tx), Some(lbl)) = (progress, label) {
                    let _ = tx.send(crate::ImportProgress { label: lbl.to_string(), done, total: tot });
                }
            }
            for items in pages {
                out.extend(items);
            }
            return Ok(out);
        }
        let mut url = v["next"].as_str().unwrap_or("").to_string();
        for _ in 1..max_pages {
            if url.is_empty() || cancel.is_some_and(|f| f.load(Ordering::Relaxed)) {
                break;
            }
            let v = self.dz.get(&url, &[]).await?;
            if let Some(items) = v["data"].as_array() {
                out.extend(items.iter().cloned());
            }
            if let (Some(tx), Some(lbl)) = (progress, label) {
                let tot = if v["next"].is_null() { out.len() } else { 0 };
                let _ = tx.send(crate::ImportProgress { label: lbl.to_string(), done: out.len(), total: tot });
            }
            match v["next"].as_str() {
                Some(n) => url = n.to_string(),
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
        let one = [("limit", "1")];
        let u_me = format!("{API}/user/{u}");
        let u_tracks = format!("{API}/user/{u}/tracks");
        let u_albums = format!("{API}/user/{u}/albums");
        let u_artists = format!("{API}/user/{u}/artists");
        let u_playlists = format!("{API}/user/{u}/playlists");
        let (me, liked, albums, artists, lists) = tokio::try_join!(
            async {
                self.dz
                    .get(&u_me, &[])
                    .await
                    .map_err(|_| anyhow!("no public Deezer profile with this number"))
            },
            self.dz.get(&u_tracks, &one),
            self.dz.get(&u_albums, &one),
            self.dz.get(&u_artists, &one),
            self.items(u_playlists, 20),
        )?;
        let account = me["name"].as_str().unwrap_or("Deezer").to_string();
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
        self.fetch_with_progress(c, None, None).await
    }

    async fn fetch_with_progress(
        &self,
        c: &Collection,
        progress: Option<&crate::flume_like::Sender<crate::ImportProgress>>,
        cancel: Option<&Arc<std::sync::atomic::AtomicBool>>,
    ) -> Result<Fetched> {
        use std::sync::atomic::Ordering;
        let u = &self.user;
        match c.kind {
            Kind::Liked => {
                let items = self.items_with_progress(format!("{API}/user/{u}/tracks"), 100, Some("IMPORT reading deezer/liked"), progress, cancel).await?;
                Ok(Fetched::Tracks(items.into_iter().filter_map(|v| serde_json::from_value::<DTrack>(v).ok()).filter_map(convert).collect()))
            }
            Kind::Playlist => Ok(Fetched::Tracks(self.dz.playlist(c.playlist_id()).await?.1)),
            Kind::Albums => {
                let items = self.items_with_progress(format!("{API}/user/{u}/albums"), 30, None, None, cancel).await?;
                let entries: Vec<(usize, String, serde_json::Value)> = items
                    .into_iter()
                    .filter_map(|a| {
                        let id = a["id"].as_u64().map(|n| n.to_string())?;
                        Some((id, a))
                    })
                    .enumerate()
                    .map(|(idx, (id, a))| (idx, id, a))
                    .collect();
                let total = entries.len();
                if let Some(tx) = progress {
                    let _ = tx.send(crate::ImportProgress { label: "IMPORT deezer/albums".into(), done: 0, total });
                }
                let dz = self.dz.clone();
                let mut slots: Vec<Option<SavedAlbum>> = vec![None; total];
                let mut done = 0usize;
                let mut stream = futures::stream::iter(entries)
                    .map(|(idx, id, a)| {
                        let dz = dz.clone();
                        async move {
                            if cancel.is_some_and(|f| f.load(Ordering::Relaxed)) {
                                return (idx, None);
                            }
                            let release = crate::providers::deezer::Release {
                                id,
                                title: a["title"].as_str().unwrap_or("").to_string(),
                                artist: a["artist"]["name"].as_str().unwrap_or("").to_string(),
                                cover_url: a["cover_xl"].as_str().or(a["cover_big"].as_str()).map(str::to_string),
                                year: None,
                                released: None,
                                kind: crate::catalog::AlbumKind::Album,
                            };
                            if release.title.is_empty() {
                                return (idx, None);
                            }
                            let tracks = dz.album_tracks(&release).await.unwrap_or_default();
                            (idx, Some(SavedAlbum { title: release.title, artist: release.artist, tracks }))
                        }
                    })
                    .buffer_unordered(12);
                while let Some((idx, album)) = stream.next().await {
                    if cancel.is_some_and(|f| f.load(Ordering::Relaxed)) {
                        break;
                    }
                    slots[idx] = album;
                    done += 1;
                    if let Some(tx) = progress {
                        let _ = tx.send(crate::ImportProgress { label: "IMPORT deezer/albums".into(), done, total });
                    }
                }
                Ok(Fetched::Albums(slots.into_iter().flatten().collect()))
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
