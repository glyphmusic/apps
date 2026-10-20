//! Last.fm profile: loved tracks become liked songs (read through the Glyph
//! server's key, or the app's own key when it has one). With an own key the
//! most played songs can also come over as a playlist (off by default). The
//! scrobble history itself stays on Last.fm.

use super::*;
use crate::providers::lastfm::LastFm;

pub struct LastFmLibrary {
    lf: Option<Arc<LastFm>>,
    user: String,
    reader: Arc<dyn LinkReader>,
}

/// The user name from a profile link (`last.fm/user/<name>`) or the name.
pub fn user_name(input: &str) -> String {
    match crate::import::parse_url(input) {
        Some(crate::import::RemoteSource::LastFmLoved(u)) => u,
        _ => input.trim().trim_start_matches('@').to_string(),
    }
}

impl LastFmLibrary {
    pub fn new(lf: Option<Arc<LastFm>>, user: String, reader: Arc<dyn LinkReader>) -> Self {
        Self { lf, user, reader }
    }

    async fn loved(&self) -> Result<Vec<Track>> {
        match &self.lf {
            Some(lf) => lf.loved_tracks(&self.user).await,
            None => Ok(self.reader.list(ImportInput::Url(format!("https://www.last.fm/user/{}", self.user))).await?.tracks),
        }
    }
}

#[async_trait]
impl LibrarySource for LastFmLibrary {
    fn service(&self) -> Service {
        Service::LastFm
    }

    async fn overview(&self) -> Result<Overview> {
        // Reading them tells whether the profile exists.
        let loved = self.loved().await?;
        let mut collections = vec![Collection::liked(Some(loved.len()))];
        if self.lf.is_some() {
            let mut top = Collection::playlist("top", "Top tracks", None, None, Some(format!("lastfm:top:{}", self.user.to_lowercase())));
            top.default_on = false;
            collections.push(top);
        }
        Ok(Overview { account: self.user.clone(), collections, notes: vec![] })
    }

    async fn fetch(&self, c: &Collection) -> Result<Fetched> {
        match (c.kind, &self.lf) {
            (Kind::Liked, _) => Ok(Fetched::Tracks(self.loved().await?)),
            (Kind::Playlist, Some(lf)) => Ok(Fetched::Tracks(lf.top_tracks(&self.user).await?)),
            (Kind::Playlist, None) => Ok(Fetched::Tracks(Vec::new())),
            (Kind::Albums, _) => Ok(Fetched::Albums(Vec::new())),
            (Kind::Artists, _) => Ok(Fetched::Artists(Vec::new())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_input() {
        assert_eq!(user_name("https://www.last.fm/user/rj/loved"), "rj");
        assert_eq!(user_name(" rj "), "rj");
    }
}
