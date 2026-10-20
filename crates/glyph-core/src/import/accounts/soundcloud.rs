//! SoundCloud library of a public profile: likes, own and liked playlists,
//! followed artists. With the app's own SoundCloud key everything is read
//! directly; without one the Glyph server reads the likes (sets come over
//! by pasting their links). Playable songs keep SoundCloud as their source
//! (the embed plays them).

use super::*;
use crate::providers::soundcloud::SoundCloud;
use anyhow::{anyhow, bail};

pub struct SoundCloudLibrary {
    sc: Option<SoundCloud>,
    profile: String,
    user: tokio::sync::OnceCell<String>,
    reader: Arc<dyn LinkReader>,
}

/// `https://soundcloud.com/<name>` from a profile link or a bare name.
pub fn profile_url(input: &str) -> Result<String> {
    let input = input.trim().trim_start_matches('@');
    if !input.is_empty() && input.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Ok(format!("https://soundcloud.com/{}", input.to_lowercase()));
    }
    match crate::import::parse_url(input) {
        Some(crate::import::RemoteSource::SoundCloud(url)) => {
            let name = url.trim_start_matches("https://soundcloud.com/").split('/').next().unwrap_or("").to_string();
            if name.is_empty() {
                bail!("paste the link to your SoundCloud profile");
            }
            Ok(format!("https://soundcloud.com/{name}"))
        }
        _ => bail!("paste the link to your SoundCloud profile (soundcloud.com/…) or your name there"),
    }
}

fn id_of(v: &serde_json::Value) -> Option<String> {
    v["id"].as_i64().map(|n| n.to_string()).or_else(|| v["id"].as_str().map(str::to_string))
}

impl SoundCloudLibrary {
    pub fn new(sc: Option<SoundCloud>, profile: String, reader: Arc<dyn LinkReader>) -> Self {
        Self { sc, profile, user: tokio::sync::OnceCell::new(), reader }
    }

    async fn user(&self, sc: &SoundCloud) -> Result<&String> {
        self.user
            .get_or_try_init(|| async {
                let v = sc.resolve_url(&self.profile).await.map_err(|_| anyhow!("no public SoundCloud profile at {}", self.profile))?;
                id_of(&v).ok_or_else(|| anyhow!("no public SoundCloud profile at {}", self.profile))
            })
            .await
    }
}

fn playlists_to_collections(items: &[serde_json::Value], followed: bool) -> Vec<Collection> {
    items
        .iter()
        .filter_map(|v| {
            // Liked playlists come wrapped as `{playlist: {...}}`.
            let p = if v["playlist"].is_object() { &v["playlist"] } else { v };
            let id = id_of(p)?;
            let name = p["title"].as_str().unwrap_or("SoundCloud set");
            let count = p["track_count"].as_u64().map(|n| n as usize);
            let cover = p["artwork_url"].as_str().map(|u| u.replace("-large.", "-t500x500."));
            let mut c = Collection::playlist(&id, name, count, cover, p["permalink_url"].as_str().map(str::to_string));
            c.followed = followed;
            Some(c)
        })
        .collect()
}

#[async_trait]
impl LibrarySource for SoundCloudLibrary {
    fn service(&self) -> Service {
        Service::SoundCloud
    }

    async fn overview(&self) -> Result<Overview> {
        let Some(sc) = &self.sc else {
            // Through the server: reading the likes tells whether the profile exists.
            let likes = self.reader.list(ImportInput::Url(format!("{}/likes", self.profile))).await?;
            let account = self.profile.trim_start_matches("https://soundcloud.com/").to_string();
            return Ok(Overview { account, collections: vec![Collection::liked(Some(likes.tracks.len()))], notes: vec!["soundcloud_sets".into()] });
        };
        let me = sc.resolve_url(&self.profile).await.map_err(|_| anyhow!("no public SoundCloud profile at {}", self.profile))?;
        if me["kind"].as_str() != Some("user") {
            bail!("this link is not a SoundCloud profile");
        }
        let id = id_of(&me).ok_or_else(|| anyhow!("no public SoundCloud profile"))?;
        let _ = self.user.set(id.clone());
        let account = me["username"].as_str().unwrap_or("SoundCloud").to_string();
        let count = |k: &str| me[k].as_u64().map(|n| n as usize);
        let own = sc.paged(&format!("/users/{id}/playlists"), 500).await.unwrap_or_default();
        let liked = sc.paged(&format!("/users/{id}/likes/playlists"), 500).await.unwrap_or_default();
        let mut collections = vec![Collection::liked(count("public_favorites_count").or(count("likes_count"))), Collection::artists(count("followings_count"))];
        collections.extend(playlists_to_collections(&own, false));
        collections.extend(playlists_to_collections(&liked, true));
        Ok(Overview { account, collections, notes: vec![] })
    }

    async fn fetch(&self, c: &Collection) -> Result<Fetched> {
        let Some(sc) = &self.sc else {
            return match c.kind {
                Kind::Liked => Ok(Fetched::Tracks(self.reader.list(ImportInput::Url(format!("{}/likes", self.profile))).await?.tracks)),
                _ => Ok(Fetched::Tracks(Vec::new())),
            };
        };
        let id = self.user(sc).await?.clone();
        match c.kind {
            Kind::Liked => Ok(Fetched::Tracks(sc.tracks_of(&format!("/users/{id}/likes/tracks")).await?)),
            Kind::Playlist => Ok(Fetched::Tracks(sc.tracks_of(&format!("/playlists/{}/tracks", c.playlist_id())).await?)),
            Kind::Artists => {
                let items = sc.paged(&format!("/users/{id}/followings"), 5000).await?;
                Ok(Fetched::Artists(items.iter().filter_map(|u| u["username"].as_str()).map(str::to_string).collect()))
            }
            Kind::Albums => Ok(Fetched::Albums(Vec::new())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_input() {
        assert_eq!(profile_url("Odesza").unwrap(), "https://soundcloud.com/odesza");
        assert_eq!(profile_url("@odesza").unwrap(), "https://soundcloud.com/odesza");
        assert_eq!(profile_url("https://soundcloud.com/odesza/likes").unwrap(), "https://soundcloud.com/odesza");
        assert!(profile_url("https://example.com/odesza").is_err());
    }

    #[test]
    fn playlist_items() {
        let items: Vec<serde_json::Value> = serde_json::from_str(
            r#"[{"id":5,"title":"Set","track_count":7,"artwork_url":"https://i1.sndcdn.com/a-large.jpg","permalink_url":"https://soundcloud.com/u/sets/set"},
                {"playlist":{"id":6,"title":"Liked set","track_count":2}}]"#,
        )
        .unwrap();
        let c = playlists_to_collections(&items, false);
        assert_eq!(c.len(), 2);
        assert_eq!((c[0].id.as_str(), c[0].count), ("pl:5", Some(7)));
        assert_eq!(c[0].cover.as_deref(), Some("https://i1.sndcdn.com/a-t500x500.jpg"));
        assert_eq!(c[1].name, "Liked set");
    }
}
