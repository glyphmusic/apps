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
            let request = client.get(url);
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
        if status.as_u16() == 206 {
            let total = resp
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_content_range_total)
                .ok_or_else(|| io_err("206 response without total length"))?;
            let mut buf = Vec::new();
            let started = std::time::Instant::now();
            (&mut resp).take(FIRST_CHUNK + 1).read_to_end(&mut buf)?;
            if buf.len() as u64 > FIRST_CHUNK { return Err(io_err("range response exceeds requested chunk")); }
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
        let mut last = None;
        for attempt in 0..RETRIES {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(300 * attempt as u64));
            }
            let transfer = glyph_protocol::network::Transfer::start();
            let request = self.client.get(&self.url);
            let chunk_bytes = glyph_protocol::network::chunk_bytes().min(CHUNK);
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
                    if resp.status().as_u16() != 206 {
                        return Err(io_err("server stopped honouring Range requests"));
                    }
                    let mut buf = Vec::with_capacity(CHUNK as usize);
                    match read_progress(&mut resp, chunk_bytes + 1, &mut buf, &transfer) {
                        Ok(_) => {
                            if buf.len() as u64 > chunk_bytes { return Err(io_err("range response exceeds requested chunk")); }
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
