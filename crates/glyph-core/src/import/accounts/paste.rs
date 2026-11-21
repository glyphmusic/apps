//! What the user copied in Spotify / YouTube Music (song and playlist links)
//! and the files they picked (Google Takeout CSVs, exports): no login.
//! Copied single songs become Liked Songs, every list link and file its own
//! playlist.

use super::*;
use crate::import::files;
use crate::import::keyless::{parse_pasted, Pasted};
use futures::stream::StreamExt;
use std::collections::HashMap;
use std::sync::Mutex;

pub struct PasteLibrary {
    service: Service,
    pasted: Pasted,
    files: Vec<PathBuf>,
    reader: Arc<dyn LinkReader>,
    /// Pre-parsed local files from `overview()` (keyed by collection id) so
    /// `fetch()` skips disk re-parsing and multi-playlist Spotify JSONs stay split.
    cached_files: Mutex<HashMap<String, ParsedList>>,
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
        Self {
            service,
            pasted: parse_pasted(text),
            files,
            reader,
            cached_files: Mutex::new(HashMap::new()),
        }
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
        let mut out = Vec::new();
        if !self.pasted.songs.is_empty() {
            out.push(Collection::liked(Some(self.pasted.songs.len())));
        }
        // Peek lightweight metadata (title, cover, count) for up to 8 pasted list links in parallel.
        let reader = self.reader.clone();
        let peek_jobs: Vec<(usize, String, crate::import::RemoteSource)> = self
            .pasted
            .lists
            .iter()
            .enumerate()
            .take(8)
            .map(|(i, (url, src))| (i, url.clone(), src.clone()))
            .collect();
        let peeked: Vec<(usize, Option<(String, Option<String>, Option<usize>)>)> = futures::stream::iter(peek_jobs)
            .map(|(i, url, src)| {
                let reader = reader.clone();
                async move { (i, reader.peek_link(&url, &src).await) }
            })
            .buffer_unordered(8)
            .collect()
            .await;
        let mut meta_by_idx: HashMap<usize, (String, Option<String>, Option<usize>)> =
            peeked.into_iter().filter_map(|(i, m)| m.map(|m| (i, m))).collect();
        for (i, (url, src)) in self.pasted.lists.iter().enumerate() {
            let (name, cover, count) = meta_by_idx
                .remove(&i)
                .map(|(n, cov, cnt)| (n, cov, cnt))
                .unwrap_or_else(|| (String::new(), None, None));
            let mut c = Collection::playlist(url, &name, count, cover, Some(url.clone()));
            c.hint = link_hint(url, src);
            out.push(c);
        }
        // Parse local files up front (milliseconds on disk) for exact track counts,
        // real titles, and splitting multi-playlist Spotify JSON exports.
        if !self.files.is_empty() {
            let paths = self.files.clone();
            let parsed = tokio::task::spawn_blocking(move || {
                paths
                    .into_iter()
                    .map(|p| {
                        let res = files::parse_file_lists(&p);
                        (p, res)
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            let mut cache = self.cached_files.lock().unwrap_or_else(|e| e.into_inner());
            for (path, res) in parsed {
                let path_str = path.to_string_lossy().to_string();
                let fallback_name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Imported playlist").to_string();
                match res {
                    Ok(lists) if lists.len() > 1 => {
                        for (idx, list) in lists.into_iter().enumerate() {
                            let id = format!("file:{path_str}#{idx}");
                            let count = Some(list.tracks.len());
                            let name = if list.name.trim().is_empty() { fallback_name.clone() } else { list.name.clone() };
                            cache.insert(id.clone(), list);
                            let mut c = Collection::playlist(&id, &name, count, None, None);
                            c.id = id;
                            out.push(c);
                        }
                    }
                    Ok(mut lists) if lists.len() == 1 => {
                        let list = lists.remove(0);
                        let id = format!("file:{path_str}");
                        let count = Some(list.tracks.len());
                        let name = if list.name.trim().is_empty() { fallback_name } else { list.name.clone() };
                        cache.insert(id.clone(), list);
                        let mut c = Collection::playlist(&id, &name, count, None, None);
                        c.id = id;
                        out.push(c);
                    }
                    _ => {
                        let mut c = Collection::playlist(&path_str, &fallback_name, None, None, None);
                        c.id = format!("file:{path_str}");
                        out.push(c);
                    }
                }
            }
        }
        Ok(Overview { account: self.service.name().into(), collections: out, notes })
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
        if c.kind == Kind::Liked {
            return Ok(Fetched::Tracks(self.reader.songs_with_progress(self.pasted.songs.clone(), progress, cancel).await));
        }
        let cached = self.cached_files.lock().unwrap_or_else(|e| e.into_inner()).get(&c.id).cloned();
        if let Some(mut list) = cached {
            if list.label == files::TAKEOUT_LABEL {
                list.tracks = self.reader.fill_takeout(list.tracks, progress, cancel).await;
            }
            return Ok(Fetched::Named(list.name, list.tracks));
        }
        if let Some(rest) = c.id.strip_prefix("file:") {
            // Resumed multi-playlist JSON (`file:<path>#<index>`)
            if let Some((path, idx_s)) = rest.rsplit_once('#') {
                if let Ok(idx) = idx_s.parse::<usize>() {
                    let path_buf = PathBuf::from(path);
                    let lists = tokio::task::spawn_blocking(move || files::parse_file_lists(&path_buf)).await??;
                    if let Some(mut list) = lists.into_iter().nth(idx) {
                        if list.label == files::TAKEOUT_LABEL {
                            list.tracks = self.reader.fill_takeout(list.tracks, progress, cancel).await;
                        }
                        return Ok(Fetched::Named(list.name, list.tracks));
                    }
                }
            }
            let list = self.reader.list_with_progress(ImportInput::File(PathBuf::from(rest)), progress, cancel).await?;
            return Ok(Fetched::Named(list.name, list.tracks));
        }
        let list = self.reader.list_with_progress(ImportInput::Url(c.playlist_id().to_string()), progress, cancel).await?;
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
