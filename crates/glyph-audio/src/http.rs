//! Seekable HTTP reader for symphonia: Range-request chunking with a
//! sequential (non-seekable) fallback for servers that ignore `Range`.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Mutex;
use std::time::Duration;

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
const FIRST_CHUNK: u64 = 64 * 1024;
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
            let mut buf = Vec::new();
            let started = std::time::Instant::now();
            (&mut resp).take(FIRST_CHUNK + 1).read_to_end(&mut buf)?;
            if buf.len() as u64 != end + 1 { return Err(io_err("incomplete audio byte range")); }
            glyph_protocol::network::observe(buf.len() as u64, started.elapsed());
            Ok(Self {
                client,
                bearer: credential(url),
                account,
                url: url.into(),
                pos: 0,
                buf,
                buf_start: 0,
                mode: Mode::Ranged { total },
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
                return Ok(Self { client, bearer: credential(url), account, url: url.into(),
                    pos: 0, buf, buf_start: 0, mode: Mode::Ranged { total: size } });
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

    fn fetch(&mut self, at: u64) -> io::Result<()> {
        let Mode::Ranged { total } = self.mode else { unreachable!() };
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
                else { glyph_protocol::network::chunk_bytes().min(CHUNK) };
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
                    let mut buf = Vec::with_capacity(CHUNK as usize);
                    match read_progress(&mut resp, limit, &mut buf, &transfer) {
                        Ok(_) => {
                            if buf.len() as u64 != limit { return Err(io_err("audio response ended before requested byte range")); }
                            glyph_protocol::network::observe(buf.len() as u64, started.elapsed());
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
                self.pos += n as u64;
                Ok(n)
            }
            Mode::Ranged { total } => {
                let total = *total;
                if self.pos >= total {
                    return Ok(0);
                }
                let buf_end = self.buf_start + self.buf.len() as u64;
                if !(self.pos >= self.buf_start && self.pos < buf_end) {
                    self.fetch(self.pos)?;
                }
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
