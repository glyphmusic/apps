//! Seekable HTTP reader for symphonia: Range-request chunking with a
//! sequential (non-seekable) fallback for servers that ignore `Range`.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Mutex;
use std::time::Duration;

#[path = "range_cache.rs"]
mod range_cache;
pub use range_cache::set_cache_dir;

/// Speculation is bounded independently of playback and never queues work.
/// The same persistent range cache serves the subsequent foreground open.
pub fn prewarm(url: String) {
    use std::collections::HashMap;
    use std::time::Instant;
    static ACTIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    static SEEN: std::sync::OnceLock<Mutex<HashMap<String, Instant>>> = std::sync::OnceLock::new();
    if ACTIVE.fetch_update(std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst,
        |n| (n < 2).then_some(n + 1)).is_err() { return; }
    let mut seen = SEEN.get_or_init(Default::default).lock().unwrap();
    seen.retain(|_, at| at.elapsed() < Duration::from_secs(300));
    if seen.contains_key(&url) {
        ACTIVE.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        return;
    }
    if seen.len() >= 64 { seen.clear(); }
    seen.insert(url.clone(), Instant::now());
    drop(seen);
    if std::thread::Builder::new().name("glyph-audio-warm".into()).spawn(move || {
        struct Active;
        impl Drop for Active { fn drop(&mut self) { ACTIVE.fetch_sub(1, std::sync::atomic::Ordering::SeqCst); } }
        let _active = Active;
        if let Ok(mut source) = HttpSource::open(&url) {
            source.prefetch_enabled = false;
            let _ = io::copy(&mut source.take(64 * 1024), &mut io::sink());
        }
    }).is_err() { ACTIVE.fetch_sub(1, std::sync::atomic::Ordering::SeqCst); }
}

static SERVER_ACCESS: std::sync::RwLock<Option<(String,String)>> = std::sync::RwLock::new(None);
pub fn set_server_access(value: Option<(String,String)>) { *SERVER_ACCESS.write().unwrap() = value; }
type AccountAccess = std::sync::Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;
static ACCOUNT_ACCESS: std::sync::RwLock<Option<AccountAccess>> = std::sync::RwLock::new(None);
pub fn set_account_access_provider(provider: AccountAccess) { *ACCOUNT_ACCESS.write().unwrap() = Some(provider); }
type LocalAccess = std::sync::Arc<dyn Fn(&str) -> Option<(Vec<(String,String)>, Option<std::path::PathBuf>)> + Send + Sync>;
static LOCAL_ACCESS: std::sync::RwLock<Option<LocalAccess>> = std::sync::RwLock::new(None);
pub fn set_local_access_provider(provider: LocalAccess) { *LOCAL_ACCESS.write().unwrap() = Some(provider); }
fn local_access(url: &str) -> Option<(Vec<(String,String)>, Option<std::path::PathBuf>)> {
    LOCAL_ACCESS.read().unwrap().as_ref()?(url)
}
fn local_headers(url: &str) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some((values, _)) = local_access(url) {
        for (name, value) in values {
            if !matches!(name.to_ascii_lowercase().as_str(), "user-agent" | "accept" | "accept-language" | "referer" | "origin") { continue; }
            if let (Ok(name), Ok(value)) = (name.parse::<reqwest::header::HeaderName>(), value.parse::<reqwest::header::HeaderValue>()) { headers.insert(name, value); }
        }
    }
    headers
}
fn account_credential(url: &str) -> Option<reqwest::header::HeaderValue> {
    let token = ACCOUNT_ACCESS.read().unwrap().as_ref()?(url)?;
    let mut value = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")).ok()?;
    value.set_sensitive(true);
    Some(value)
}
fn credential(url: &str) -> Option<reqwest::header::HeaderValue> {
    let guard = SERVER_ACCESS.read().unwrap();
    let (base,token) = guard.as_ref()?;
    let (base,url) = (reqwest::Url::parse(base).ok()?,reqwest::Url::parse(url).ok()?);
    if base.origin() != url.origin() || !url.path().starts_with(&format!("{}/",base.path().trim_end_matches('/'))) { return None; }
    let mut value = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")).ok()?;
    value.set_sensitive(true);
    Some(value)
}

const CHUNK: u64 = 384 * 1024;
// About 12 seconds at 160 kbps: covers companion latency during the first refill.
const FIRST_CHUNK: u64 = 256 * 1024;
const RETRIES: usize = 3;

pub fn client() -> reqwest::Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .no_gzip()
        .user_agent(concat!("Glyph/", env!("CARGO_PKG_VERSION")))
        .build()
}

/// Parse `Content-Range: bytes 0-99/1234` -> total length.
pub fn parse_content_range_total(v: &str) -> Option<u64> {
    let total = v.rsplit('/').next()?.trim();
    if total == "*" {
        None
    } else {
        total.parse().ok()
    }
}

fn range_bounds(v: &str) -> Option<(u64, u64, u64)> {
    let (bounds, total) = v.strip_prefix("bytes ")?.split_once('/')?;
    let (start, end) = bounds.split_once('-')?;
    let (start, end, total) = (start.parse().ok()?, end.parse().ok()?, total.parse().ok()?);
    (start <= end && end < total).then_some((start, end, total))
}

/// `Range` header value for `[start, start + len)` clipped to `total`.
pub fn range_header(start: u64, len: u64, total: Option<u64>) -> String {
    let mut end = start + len - 1;
    if let Some(t) = total {
        end = end.min(t.saturating_sub(1));
    }
    format!("bytes={start}-{end}")
}

enum Mode {
    Ranged { total: u64 },
    Sequential { body: Mutex<reqwest::blocking::Response>, total: Option<u64> },
}

pub struct HttpSource {
    client: reqwest::blocking::Client,
    url: String,
    pos: u64,
    buf: Vec<u8>,
    buf_start: u64,
    mode: Mode,
    bearer: Option<reqwest::header::HeaderValue>,
    account: Option<reqwest::header::HeaderValue>,
    ahead: Option<(u64, flume::Receiver<io::Result<Vec<u8>>>)>,
    // Keep the first range open: decoding must not wait for its entire body.
    initial: Option<(reqwest::blocking::Response, u64)>,
    cache: Option<range_cache::Entry>,
    prefetch_enabled: bool,
}

fn io_err(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

fn read_progress(reader: &mut impl Read, limit: u64, out: &mut Vec<u8>, transfer: &glyph_protocol::network::Transfer) -> io::Result<()> {
    let mut body=reader.take(limit);
    let mut block=[0u8;16*1024];
    loop {
        let n=body.read(&mut block)?;
        if n==0 {return Ok(());}
        transfer.progress();
        out.extend_from_slice(&block[..n]);
    }
}

impl HttpSource {
    pub fn open(url: &str) -> io::Result<Self> {
        let bearer = credential(url);
        let account = account_credential(url);
        let client = if bearer.is_some() || account.is_some() { reqwest::blocking::Client::builder().redirect(reqwest::redirect::Policy::none()).connect_timeout(Duration::from_secs(10)).build() } else { client() }.map_err(io_err)?;
        if let Some(cache) = range_cache::Entry::open(url, None) {
            if let Some(buf) = cache.read(0, FIRST_CHUNK) {
                return Ok(Self { client, url: url.into(), pos: 0, buf, buf_start: 0,
                    mode: Mode::Ranged { total: cache.total }, bearer, account, ahead: None,
                    initial: None, cache: Some(cache), prefetch_enabled: true });
            }
        }
        let mut last = None;
        for attempt in 0..RETRIES {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(300 * attempt as u64));
            }
            let request = client.get(url).headers(local_headers(url));
            let request = if let Some(value) = &account { request.header(reqwest::header::AUTHORIZATION, value.clone()) } else { request };
            let request = if let Some(value) = &bearer { request.header("x-glyph-server-authorization",value.clone()) } else { request };
            match request
                .header(reqwest::header::RANGE, range_header(0, FIRST_CHUNK, None))
                .timeout(Duration::from_secs(20))
                .send()
            {
                Ok(resp) => return Self::from_first_response(client, url, resp, account),
                Err(e) => last = Some(e),
            }
        }
        Err(io_err(last.unwrap()))
    }

    fn from_first_response(
        client: reqwest::blocking::Client,
        url: &str,
        mut resp: reqwest::blocking::Response,
        account: Option<reqwest::header::HeaderValue>,
    ) -> io::Result<Self> {
        let status = resp.status();
        #[cfg(test)]
        eprintln!("audio first response: {status}, range={:?}", resp.headers().get(reqwest::header::CONTENT_RANGE));
        if status.as_u16() == 206 {
            let (start, end, total) = resp
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(range_bounds)
                .ok_or_else(|| io_err("invalid audio Content-Range"))?;
            if start != 0 || end >= FIRST_CHUNK { return Err(io_err("audio response returned the wrong byte range")); }
            let buf = Vec::new();
            Ok(Self {
                client,
                bearer: credential(url),
                account,
                url: url.into(),
                pos: 0,
                buf,
                buf_start: 0,
                mode: Mode::Ranged { total },
                ahead: None,
                initial: Some((resp, end + 1)),
                cache: range_cache::Entry::open(url, Some(total)),
                prefetch_enabled: true,
            })
        } else if status.is_success() {
            let total = resp.content_length();
            // Small full responses (notably 30-second MP3 previews) fit in
            // memory. Keep them seekable so header probing and empty ID3 tags
            // behave exactly as they do on ranged or local audio.
            if let Some(size) = total.filter(|n| *n > 0 && *n <= 2 * 1024 * 1024) {
                let mut buf = Vec::with_capacity(size as usize);
                (&mut resp).take(size + 1).read_to_end(&mut buf)?;
                if buf.len() as u64 != size { return Err(io_err("incomplete full audio response")); }
                let cache = range_cache::Entry::open(url, Some(size));
                if let Some(cache) = &cache { cache.write(0, &buf); }
                return Ok(Self { client, bearer: credential(url), account, url: url.into(),
                    pos: 0, buf, buf_start: 0, ahead: None, initial: None, cache, prefetch_enabled: true, mode: Mode::Ranged { total: size } });
            }
            Ok(Self {
                client,
                bearer: credential(url),
                url: url.into(),
                pos: 0,
                buf: Vec::new(),
                buf_start: 0,
                mode: Mode::Sequential { body: Mutex::new(resp), total },
                account,
                ahead: None,
                initial: None,
                cache: total.and_then(|size| range_cache::Entry::open(url, Some(size))),
                prefetch_enabled: true,
            })
        } else {
            Err(io_err(format!("HTTP {status}")))
        }
    }

    pub fn is_seekable(&self) -> bool {
        matches!(self.mode, Mode::Ranged { .. })
    }

    pub fn byte_len(&self) -> Option<u64> {
        match &self.mode {
            Mode::Ranged { total } => Some(*total),
            Mode::Sequential { total, .. } => *total,
        }
    }

    // One bounded block ahead, independent of the decoder. No waiting or
    // network work occurs on the audio callback. Seeking discards stale results.
    fn prefetch(&mut self) {
        if !self.prefetch_enabled { return; }
        let Mode::Ranged { total } = self.mode else { return };
        let at = self.initial.as_ref().map(|(_, end)| *end)
            .unwrap_or(self.buf_start + self.buf.len() as u64);
        if at >= total || self.ahead.is_some() { return; }
        let mut worker = Self {
            client: self.client.clone(), url: self.url.clone(), pos: at,
            buf: Vec::new(), buf_start: at, mode: Mode::Ranged { total },
            bearer: self.bearer.clone(), account: self.account.clone(), ahead: None,
            initial: None,
            cache: self.cache.clone(),
            prefetch_enabled: true,
        };
        let (tx, rx) = flume::bounded(1);
        if std::thread::Builder::new().name("glyph-audio-prefetch".into()).spawn(move || {
            let result = worker.fetch(at, false).map(|()| worker.buf);
            let _ = tx.send(result);
        }).is_ok() { self.ahead = Some((at, rx)); }
    }

    fn read_initial(&mut self, requested: usize) -> io::Result<()> {
        let buf_end = self.buf_start + self.buf.len() as u64;
        if self.pos != buf_end { return Ok(()); }
        if let Some((body, end)) = &mut self.initial {
            let mut block = [0u8; 16 * 1024];
            let limit = (*end - buf_end).min(block.len() as u64).min(requested as u64) as usize;
            let started = std::time::Instant::now();
            let n = body.read(&mut block[..limit])?;
            if n == 0 { return Err(io_err("incomplete audio byte range")); }
            self.buf.extend_from_slice(&block[..n]);
            if let Some(cache) = &self.cache { cache.write(buf_end, &block[..n]); }
            glyph_protocol::network::observe(n as u64, started.elapsed());
            if self.buf_start + self.buf.len() as u64 == *end { self.initial = None; }
        }
        Ok(())
    }

    fn fetch(&mut self, at: u64, progressive: bool) -> io::Result<()> {
        self.initial = None;
        let Mode::Ranged { total } = self.mode else { unreachable!() };
        if let Some(bytes) = self.cache.as_ref().and_then(|cache| cache.read(at, CHUNK)) {
            self.buf = bytes;
            self.buf_start = at;
            return Ok(());
        }
        if let Some((_, Some(path))) = local_access(&self.url) {
            if let Ok(mut file) = std::fs::File::open(path) {
                if file.metadata().is_ok_and(|m| m.len() == total) {
                    file.seek(SeekFrom::Start(at))?;
                    let mut bytes = Vec::new();
                    file.take(CHUNK.min(total.saturating_sub(at))).read_to_end(&mut bytes)?;
                    self.buf = bytes;
                    self.buf_start = at;
                    return Ok(());
                }
            }
        }
        let mut last = None;
        for attempt in 0..RETRIES {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(300 * attempt as u64));
            }
            let transfer = glyph_protocol::network::Transfer::start();
            let request = self.client.get(&self.url).headers(local_headers(&self.url));
            // Companion requests have a fixed extraction/tunnel latency. A
            // small measured first request must not shrink later refills to
            // 32 KiB and repeat that latency for every MP4 segment.
            let chunk_bytes = if self.url.contains("/v1/audio/youtube/") { 1024 * 1024 }
                else { CHUNK };
            #[cfg(test)]
            eprintln!("audio fetch: at={at}, chunk={chunk_bytes}, total={total}");
            let started = std::time::Instant::now();
            let request = if let Some(value) = &self.account { request.header(reqwest::header::AUTHORIZATION, value.clone()) } else { request };
            let request = if let Some(value) = &self.bearer { request.header("x-glyph-server-authorization",value.clone()) } else { request };
            let r = request
                .header(reqwest::header::RANGE, range_header(at, chunk_bytes, Some(total)))
                .timeout(Duration::from_secs(20))
                .send()
                .and_then(|r| r.error_for_status());
            match r {
                Ok(mut resp) => {
                    let mut limit = chunk_bytes.min(total.saturating_sub(at));
                    if resp.status().as_u16() == 200 {
                        // Some proxies serve a full cached object after an
                        // earlier ranged response. Keep our logical position:
                        // discard exactly the prefix, never decode it twice.
                        if resp.content_length().is_some_and(|n| n != total) {
                            return Err(io_err("audio length changed during playback"));
                        }
                        let skipped = io::copy(&mut resp.by_ref().take(at), &mut io::sink())?;
                        if skipped != at { return Err(io_err("full audio response ended before requested position")); }
                    } else if resp.status().as_u16() != 206 {
                        return Err(io_err("unexpected audio response status"));
                    } else {
                        let bounds = resp.headers().get(reqwest::header::CONTENT_RANGE)
                            .and_then(|h| h.to_str().ok()).and_then(range_bounds);
                        let Some((start, end, size)) = bounds else { return Err(io_err("invalid audio Content-Range")); };
                        if start != at || size != total || end - start + 1 > limit {
                            return Err(io_err("audio response returned the wrong byte range"));
                        }
                        limit = end - start + 1;
                    }
                    if progressive {
                        self.buf.clear();
                        self.buf_start = at;
                        self.initial = Some((resp, at + limit));
                        return Ok(());
                    }
                    let mut buf = Vec::with_capacity(CHUNK as usize);
                    match read_progress(&mut resp, limit, &mut buf, &transfer) {
                        Ok(_) => {
                            if buf.len() as u64 != limit { return Err(io_err("audio response ended before requested byte range")); }
                            glyph_protocol::network::observe(buf.len() as u64, started.elapsed());
                            if let Some(cache) = &self.cache { cache.write(at, &buf); }
                            self.buf = buf;
                            self.buf_start = at;
                            return Ok(());
                        }
                        Err(e) => last = Some(io_err(e)),
                    }
                }
                Err(e) => last = Some(io_err(e)),
            }
        }
        Err(last.unwrap())
    }
}

impl Read for HttpSource {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        match &self.mode {
            Mode::Sequential { body, .. } => {
                let n = body.lock().unwrap().read(out)?;
                if let Some(cache) = &self.cache { cache.write(self.pos, &out[..n]); }
                self.pos += n as u64;
                Ok(n)
            }
            Mode::Ranged { total } => {
                let total = *total;
                if self.pos >= total {
                    return Ok(0);
                }
                self.read_initial(out.len())?;
                let buf_end = self.buf_start + self.buf.len() as u64;
                if !(self.pos >= self.buf_start && self.pos < buf_end) {
                    let cached = self.ahead.take().and_then(|(at, rx)| {
                        (at == self.pos).then(|| rx.recv().ok()).flatten()
                    });
                    if let Some(Ok(bytes)) = cached {
                        self.buf_start = self.pos;
                        self.buf = bytes;
                    } else { self.fetch(self.pos, true)?; }
                    self.read_initial(out.len())?;
                }
                self.prefetch();
                let off = (self.pos - self.buf_start) as usize;
                let n = out.len().min(self.buf.len().saturating_sub(off));
                if n == 0 {
                    return Ok(0);
                }
                out[..n].copy_from_slice(&self.buf[off..off + n]);
                self.pos += n as u64;
                Ok(n)
            }
        }
    }
}

impl Seek for HttpSource {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(p) => p as i128,
            SeekFrom::Current(d) => self.pos as i128 + d as i128,
            SeekFrom::End(d) => match self.byte_len() {
                Some(t) => t as i128 + d as i128,
                None => return Err(io::Error::from(io::ErrorKind::Unsupported)),
            },
        };
        if target < 0 {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        let target = target as u64;
        if !self.is_seekable() && target != self.pos {
            return Err(io::Error::from(io::ErrorKind::Unsupported));
        }
        self.pos = target;
        Ok(target)
    }
}

/// Either an HTTP stream or a local file (`file://` URL or plain path).
pub enum AudioSource {
    Http(HttpSource),
    File(std::fs::File),
}

impl AudioSource {
    pub fn open(url: &str) -> io::Result<Self> {
        if url.starts_with("http://") || url.starts_with("https://") {
            HttpSource::open(url).map(Self::Http)
        } else {
            let path = url.strip_prefix("file://").unwrap_or(url);
            // `file:///C:/x` -> `/C:/x` -> `C:/x` on Windows
            let b = path.as_bytes();
            let path = if cfg!(windows) && b.len() > 2 && b[0] == b'/' && b[2] == b':' {
                &path[1..]
            } else {
                path
            };
            std::fs::File::open(path).map(Self::File)
        }
    }
}

impl Read for AudioSource {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Http(h) => h.read(b),
            Self::File(f) => f.read(b),
        }
    }
}

impl Seek for AudioSource {
    fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
        match self {
            Self::Http(h) => h.seek(p),
            Self::File(f) => f.seek(p),
        }
    }
}

impl symphonia::core::io::MediaSource for AudioSource {
    fn is_seekable(&self) -> bool {
        match self {
            Self::Http(h) => h.is_seekable(),
            Self::File(_) => true,
        }
    }
    fn byte_len(&self) -> Option<u64> {
        match self {
            Self::Http(h) => h.byte_len(),
            Self::File(f) => f.metadata().ok().map(|m| m.len()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    #[test]
    fn content_range_parsing() {
        assert_eq!(parse_content_range_total("bytes 0-99/1234"), Some(1234));
        assert_eq!(parse_content_range_total("bytes 0-99/*"), None);
        assert_eq!(parse_content_range_total("garbage"), None);
    }

    #[test]
    fn range_header_clips() {
        assert_eq!(range_header(0, 100, None), "bytes=0-99");
        assert_eq!(range_header(950, 100, Some(1000)), "bytes=950-999");
    }

    /// Live check against the real Deezer preview CDN (network test).
    #[test]
    #[ignore = "needs network"]
    fn live_deezer_preview() {
        use symphonia::core::io::MediaSource;
        let api = reqwest::blocking::get("https://api.deezer.com/search?q=daft+punk&limit=1").unwrap();
        let v: serde_json::Value = api.json().unwrap();
        let url = v["data"][0]["preview"].as_str().unwrap().to_string();
        println!("url = {url}");
        let mut src = AudioSource::open(&url).expect("open failed");
        println!("seekable={} byte_len={:?}", src.is_seekable(), src.byte_len());
        let mut b = [0u8; 16];
        src.read_exact(&mut b).unwrap();
        println!("first 16 bytes = {:02X?}", b);
    }

    /// Tiny Range-capable server; verifies seeking + chunk refetch.
    #[test]
    fn continues_when_proxy_switches_from_range_to_full_body() {
        let data: Vec<u8> = (0..FIRST_CHUNK * 3 + 19).map(|i| (i % 251) as u8).collect();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let served = data.clone();
        let server = std::thread::spawn(move || {
            for (index, stream) in listener.incoming().take(2).enumerate() {
                let mut stream = stream.unwrap();
                let mut request = [0u8; 4096];
                stream.read(&mut request).unwrap();
                let (head, body) = if index == 0 {
                    (format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", FIRST_CHUNK - 1, served.len(), FIRST_CHUNK), &served[..FIRST_CHUNK as usize])
                } else {
                    (format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", served.len()), served.as_slice())
                };
                stream.write_all(head.as_bytes()).unwrap();
                stream.write_all(body).unwrap();
            }
        });
        let mut source = HttpSource::open(&format!("http://{address}/song")).unwrap();
        source.seek(SeekFrom::End(-37)).unwrap();
        let mut tail = Vec::new();
        source.read_to_end(&mut tail).unwrap();
        assert_eq!(tail, data[data.len() - 37..]);
        server.join().unwrap();
    }

    #[test]
    fn distant_seek_reads_before_its_range_finishes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (continue_tx, continue_rx) = std::sync::mpsc::channel();
        let (prefix_tx, prefix_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            stream.read(&mut request).unwrap();
            write!(stream, "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-9/100\r\nContent-Length: 10\r\nConnection: close\r\n\r\n0123456789").unwrap();
            drop(stream);
            let (mut stream, _) = listener.accept().unwrap();
            let n = stream.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..n]).contains("bytes=90-99"));
            write!(stream, "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 90-99/100\r\nContent-Length: 10\r\nConnection: close\r\n\r\nabc").unwrap();
            stream.flush().unwrap();
            let released = continue_rx.recv_timeout(Duration::from_secs(5)).is_ok();
            stream.write_all(b"defghij").unwrap();
            released
        });
        let client = std::thread::spawn(move || {
            let mut source = HttpSource::open(&format!("http://{address}/seek")).unwrap();
            source.seek(SeekFrom::Start(90)).unwrap();
            let mut prefix = [0; 3];
            source.read_exact(&mut prefix).unwrap();
            prefix_tx.send(prefix).unwrap();
            let mut tail = Vec::new();
            source.read_to_end(&mut tail).unwrap();
            tail
        });
        let prefix = prefix_rx.recv_timeout(Duration::from_secs(2));
        let _ = continue_tx.send(());
        assert_eq!(client.join().unwrap(), b"defghij");
        assert!(server.join().unwrap());
        assert_eq!(prefix.unwrap(), *b"abc");
    }

    #[test]
    fn first_range_decodes_before_download_finishes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (continue_tx, continue_rx) = std::sync::mpsc::channel();
        let (prefix_tx, prefix_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            stream.read(&mut request).unwrap();
            write!(stream, "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-9/10\r\nContent-Length: 10\r\nConnection: close\r\n\r\nabc").unwrap();
            stream.flush().unwrap();
            // A whole-range buffer would deadlock until this timeout.
            let released = continue_rx.recv_timeout(Duration::from_secs(5)).is_ok();
            stream.write_all(b"defghij").unwrap();
            released
        });
        let client = std::thread::spawn(move || {
            let mut source = HttpSource::open(&format!("http://{address}/slow-audio")).unwrap();
            let mut prefix = [0; 3];
            source.read_exact(&mut prefix).unwrap();
            prefix_tx.send(prefix).unwrap();
            // Already received bytes remain seekable during the download.
            source.seek(SeekFrom::Start(0)).unwrap();
            let mut all = Vec::new();
            source.read_to_end(&mut all).unwrap();
            all
        });
        let prefix = prefix_rx.recv_timeout(Duration::from_secs(2));
        let _ = continue_tx.send(());
        let all = client.join().unwrap();
        let released = server.join().unwrap();
        assert_eq!(prefix.unwrap(), *b"abc");
        assert!(released);
        assert_eq!(all, b"abcdefghij");
    }

    #[test]
    fn small_full_response_is_seekable_for_header_probing() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let bytes = b"ID3\x04\0\0\0\0\0\0audio payload";
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            stream.read(&mut request).unwrap();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len()).unwrap();
            stream.write_all(bytes).unwrap();
        });
        let mut source = HttpSource::open(&format!("http://{address}/preview")).unwrap();
        assert!(source.is_seekable());
        let mut header = [0u8; 10];
        source.read_exact(&mut header).unwrap();
        source.seek(SeekFrom::Start(0)).unwrap();
        let mut all = Vec::new();
        source.read_to_end(&mut all).unwrap();
        assert_eq!(all, bytes);
        server.join().unwrap();
    }

    #[test]
    fn switches_later_ranges_to_completed_local_cache() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let url = format!("http://{address}/cached-audio");
        let data: Vec<u8> = (0..FIRST_CHUNK * 3 + 19).map(|i| (i % 251) as u8).collect();
        let path = std::env::temp_dir().join(format!("glyph-range-cache-{}-{}.bin", std::process::id(), address.port()));
        std::fs::write(&path, &data).unwrap();
        let ready = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (lookup_url, lookup_path, lookup_ready) = (url.clone(), path.clone(), ready.clone());
        set_local_access_provider(std::sync::Arc::new(move |requested| (requested == lookup_url).then(|| (
            vec![("User-Agent".into(), "glyph-cache-test".into()), ("Authorization".into(), "must-not-forward".into())],
            lookup_ready.load(std::sync::atomic::Ordering::Relaxed).then(|| lookup_path.clone()),
        ))));
        let served = data.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let n = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..n]).to_ascii_lowercase();
            assert!(request.contains("user-agent: glyph-cache-test"));
            assert!(!request.contains("authorization:"));
            write!(stream, "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", FIRST_CHUNK - 1, served.len(), FIRST_CHUNK).unwrap();
            stream.write_all(&served[..FIRST_CHUNK as usize]).unwrap();
        });
        let mut source = HttpSource::open(&url).unwrap();
        server.join().unwrap();
        ready.store(true, std::sync::atomic::Ordering::Relaxed);
        source.seek(SeekFrom::End(-37)).unwrap();
        let mut tail = Vec::new();
        source.read_to_end(&mut tail).unwrap();
        assert_eq!(tail, data[data.len() - 37..]);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn prefetches_before_boundary_without_losing_or_repeating_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let data: Vec<u8> = (0..FIRST_CHUNK + CHUNK).map(|i| (i % 251) as u8).collect();
        let served = data.clone();
        let (requested, received) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 4096];
                stream.read(&mut request).unwrap();
                let start = if index == 0 { 0 } else { FIRST_CHUNK as usize };
                let end = if index == 0 { FIRST_CHUNK as usize } else { served.len() };
                if index == 1 {
                    requested.send(()).unwrap();
                    // Model a slow tunnel while the decoder still owns its first block.
                    std::thread::sleep(Duration::from_millis(150));
                }
                write!(stream, "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", start, end-1, served.len(), end-start).unwrap();
                stream.write_all(&served[start..end]).unwrap();
            }
        });
        let mut source = HttpSource::open(&format!("http://{address}/song")).unwrap();
        let mut first = [0u8; 1];
        source.read_exact(&mut first).unwrap();
        // The refill starts before the first block is consumed.
        received.recv_timeout(Duration::from_secs(2)).unwrap();
        let mut actual = first.to_vec();
        source.read_to_end(&mut actual).unwrap();
        assert_eq!(actual, data);
        server.join().unwrap();
    }

    /// Tiny Range-capable server; verifies seeking + chunk refetch.
    #[test]
    fn ranged_read_and_seek() {
        let data: Vec<u8> = (0..(CHUNK * 2 + 1234)).map(|i| (i % 251) as u8).collect();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let served = data.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut s = stream.unwrap();
                let mut req = vec![0u8; 4096];
                let n = s.read(&mut req).unwrap();
                let req = String::from_utf8_lossy(&req[..n]).to_lowercase();
                let r = req.lines().find(|l| l.starts_with("range:")).unwrap().to_string();
                let r = r.trim_start_matches("range: bytes=");
                let (a, b) = r.split_once('-').unwrap();
                let a: usize = a.parse().unwrap();
                let b: usize = b.trim().parse::<usize>().unwrap().min(served.len() - 1);
                let body = &served[a..=b];
                let head = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {a}-{b}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    served.len(),
                    body.len()
                );
                s.write_all(head.as_bytes()).unwrap();
                s.write_all(body).unwrap();
            }
        });
        let mut src = HttpSource::open(&format!("http://127.0.0.1:{port}/a.mp3")).unwrap();
        assert!(src.is_seekable());
        assert_eq!(src.byte_len(), Some(data.len() as u64));
        let mut b = [0u8; 10];
        src.read_exact(&mut b).unwrap();
        assert_eq!(&b, &data[..10]);
        let off = CHUNK + 500;
        src.seek(SeekFrom::Start(off)).unwrap();
        src.read_exact(&mut b).unwrap();
        assert_eq!(&b, &data[off as usize..off as usize + 10]);
        src.seek(SeekFrom::End(-5)).unwrap();
        let mut tail = Vec::new();
        src.read_to_end(&mut tail).unwrap();
        assert_eq!(tail, data[data.len() - 5..]);
    }
}
