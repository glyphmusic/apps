//! What the user copied in Spotify / YouTube Music (song and playlist links)
//! and the files they picked (Google Takeout CSVs, exports): no login.
//! Copied single songs become Liked Songs, every list link and file its own
//! playlist.

use super::*;
use crate::import::keyless::{parse_pasted, Pasted};

pub struct PasteLibrary {
    service: Service,
    pasted: Pasted,
    files: Vec<PathBuf>,
    reader: Arc<dyn LinkReader>,
}

/// `spotify/playlist · 37i9dQZF…` for a list whose name is not read yet.
fn link_hint(url: &str, src: &crate::import::RemoteSource) -> String {
    let tail = url.trim_end_matches('/').rsplit('/').next().unwrap_or(url);
    let tail = tail.split('?').next().unwrap_or(tail);
    let short: String = tail.chars().take(10).collect();
    format!("{} · {short}{}", src.label(), if tail.chars().count() > 10 { "…" } else { "" })
}

impl PasteLibrary {
    pub fn new(service: Service, text: &str, files: Vec<PathBuf>, reader: Arc<dyn LinkReader>) -> Self {
        Self { service, pasted: parse_pasted(text), files, reader }
    }

    pub fn is_empty(&self) -> bool {
        self.pasted.is_empty() && self.files.is_empty()
    }

    /// The collections, without reading anything.
    pub fn collections(&self) -> Vec<Collection> {
        let mut out = Vec::new();
        if !self.pasted.songs.is_empty() {
            out.push(Collection::liked(Some(self.pasted.songs.len())));
        }
        for (url, src) in &self.pasted.lists {
            let mut c = Collection::playlist(url, "", None, None, Some(url.clone()));
            c.hint = link_hint(url, src);
            out.push(c);
        }
        for f in &self.files {
            let name = f.file_stem().and_then(|s| s.to_str()).unwrap_or("Imported playlist");
            let mut c = Collection::playlist(&f.to_string_lossy(), name, None, None, None);
            c.id = format!("file:{}", f.to_string_lossy());
            out.push(c);
        }
        out
    }
}

#[async_trait]
impl LibrarySource for PasteLibrary {
    fn service(&self) -> Service {
        self.service
    }

    async fn overview(&self) -> Result<Overview> {
        let mut notes = Vec::new();
        if self.service == Service::Spotify && self.pasted.lists.iter().any(|(_, s)| matches!(s, crate::import::RemoteSource::SpotifyPlaylist(_))) {
            notes.push("spotify_long_lists".into());
        }
        Ok(Overview { account: self.service.name().into(), collections: self.collections(), notes })
    }

    async fn fetch(&self, c: &Collection) -> Result<Fetched> {
        if c.kind == Kind::Liked {
            return Ok(Fetched::Tracks(self.reader.songs(self.pasted.songs.clone()).await));
        }
        let input = match c.id.strip_prefix("file:") {
            Some(path) => ImportInput::File(PathBuf::from(path)),
            None => ImportInput::Url(c.playlist_id().to_string()),
        };
        let list = self.reader.list(input).await?;
        Ok(Fetched::Named(list.name, list.tracks))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoReader;
    #[async_trait]
    impl LinkReader for NoReader {
        async fn list(&self, _: ImportInput) -> Result<ParsedList> {
            anyhow::bail!("offline")
        }
        async fn songs(&self, _: Vec<SongLink>) -> Vec<Track> {
            Vec::new()
        }
    }

    #[test]
    fn collections_of_pasted_text() {
        let text = "https://open.spotify.com/track/3XmwAZpIMNpTJy8faw3ajD\nhttps://open.spotify.com/track/2Foc5Q5nqNiosCNqttzHof\n\
                    https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=x";
        let lib = PasteLibrary::new(Service::Spotify, text, vec![PathBuf::from("C:/x/Liked music.csv")], Arc::new(NoReader));
        let c = lib.collections();
        assert_eq!(c.len(), 3);
        assert_eq!((c[0].kind, c[0].count), (Kind::Liked, Some(2)));
        assert_eq!(c[1].origin.as_deref(), Some("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=x"));
        assert!(c[1].name.is_empty());
        assert_eq!(c[1].hint, "spotify/playlist · 37i9dQZF1D…");
        assert_eq!(c[2].name, "Liked music");
        assert!(c[2].id.starts_with("file:"));
        assert!(PasteLibrary::new(Service::Spotify, "nothing here", vec![], Arc::new(NoReader)).is_empty());
    }
}
