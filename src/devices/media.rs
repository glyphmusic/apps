//! Small HTTP server that hands songs to network devices: local files
//! (downloads, the playback cache) and remote streams, which it relays as
//! plain HTTP (TVs and speakers often cannot open HTTPS, long signed URLs or
//! URLs without a file extension). One random path per song; Range
//! requests work, so devices can seek.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{IpAddr, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone)]
enum Source {
    File(PathBuf),
    Remote(String),
}

#[derive(Debug, Clone)]
struct Entry {
    source: Source,
    mime: String,
}

pub struct MediaServer {
    port: u16,
    entries: Arc<Mutex<HashMap<String, Entry>>>,
}

/// File extension devices expect for `mime`.
pub fn ext_for(mime: &str) -> &'static str {
    match mime.split(';').next().unwrap_or("").trim() {
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/mp4" | "audio/aac" | "audio/x-m4a" | "audio/m4a" => "m4a",
        "audio/webm" => "webm",
        "audio/ogg" | "audio/vorbis" => "ogg",
        "audio/opus" => "opus",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        _ => "mp3",
    }
}

/// Best guess at a stream's type: the given one, a `mime=` query parameter
/// (YouTube), or the file extension.
pub fn guess_mime(url: &str, given: Option<&str>) -> String {
    if let Some(m) = given.filter(|m| m.starts_with("audio/")) {
        return m.split(';').next().unwrap_or(m).trim().to_string();
    }
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    for kv in query.split('&') {
        if let Some(v) = kv.strip_prefix("mime=") {
            let v = v.replace("%2F", "/").replace("%2f", "/");
            if v.starts_with("audio/") {
                return v;
            }
        }
    }
    let ext = path.rsplit('/').next().and_then(|f| f.rsplit_once('.')).map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "m4a" | "mp4" | "aac" => "audio/mp4",
        "webm" => "audio/webm",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        _ => "audio/mpeg",
    }
    .to_string()
}

/// This computer's address on the route to `ip`.
pub fn local_ip_towards(ip: IpAddr) -> Option<IpAddr> {
    let sock = UdpSocket::bind(if ip.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" }).ok()?;
    sock.connect((ip, 9)).ok()?;
    Some(sock.local_addr().ok()?.ip())
}

impl MediaServer {
    pub fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind(("0.0.0.0", 0))?;
        let port = listener.local_addr()?.port();
        let entries: Arc<Mutex<HashMap<String, Entry>>> = Default::default();
        let e2 = entries.clone();
        std::thread::Builder::new().name("glyph-cast-files".into()).spawn(move || {
            for s in listener.incoming().flatten() {
                let entries = e2.clone();
                let _ = std::thread::Builder::new().name("glyph-cast-file".into()).spawn(move || {
                    if let Err(e) = serve(s, &entries) {
                        tracing::debug!("device media request: {e}");
                    }
                });
            }
        })?;
        Ok(Self { port, entries })
    }

    /// A plain-HTTP URL the device at `device` can fetch `src` (URL or
    /// local path) from.
    pub fn url_for(&self, src: &str, mime: &str, device: IpAddr) -> Option<String> {
        let source = if src.starts_with("http://") || src.starts_with("https://") {
            Source::Remote(src.to_string())
        } else {
            let p = PathBuf::from(src);
            if !p.is_file() {
                return None;
            }
            Source::File(p)
        };
        let id = glyph_core::random_token();
        {
            let mut e = self.entries.lock().unwrap();
            if e.len() > 50 {
                e.clear();
            }
            e.insert(id.clone(), Entry { source, mime: mime.to_string() });
        }
        let ip = local_ip_towards(device)?;
        let host = if ip.is_ipv6() { format!("[{ip}]") } else { ip.to_string() };
        Some(format!("http://{host}:{}/{id}.{}", self.port, ext_for(mime)))
    }
}

/// `bytes=a-b` -> (a, b).
fn parse_range(v: &str) -> Option<(u64, Option<u64>)> {
    let (a, b) = v.trim().strip_prefix("bytes=")?.split(',').next()?.split_once('-')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()))
}

const DLNA: &str = "transferMode.dlna.org: Streaming\r\ncontentFeatures.dlna.org: DLNA.ORG_OP=01;DLNA.ORG_CI=0;DLNA.ORG_FLAGS=01700000000000000000000000000000\r\n";

fn serve(s: TcpStream, entries: &Mutex<HashMap<String, Entry>>) -> std::io::Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut r = BufReader::new(s.try_clone()?);
    let mut first = String::new();
    r.read_line(&mut first)?;
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let path = parts.next().unwrap_or("/").trim_start_matches('/').to_string();
    let mut range_raw: Option<String> = None;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.trim().eq_ignore_ascii_case("range") {
                range_raw = Some(v.trim().to_string());
            }
        }
    }
    let id = path.split(['.', '?']).next().unwrap_or("");
    let entry = entries.lock().unwrap().get(id).cloned();
    let mut out = s;
    out.set_write_timeout(Some(Duration::from_secs(30)))?;
    let Some(entry) = entry else {
        return out.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    };
    let head_only = method.eq_ignore_ascii_case("HEAD");
    match entry.source {
        Source::File(p) => serve_file(out, &p, &entry.mime, range_raw.as_deref().and_then(parse_range), head_only),
        Source::Remote(u) => proxy(out, &u, &entry.mime, range_raw, head_only),
    }
}

fn serve_file(mut out: TcpStream, file: &std::path::Path, mime: &str, range: Option<(u64, Option<u64>)>, head_only: bool) -> std::io::Result<()> {
    let mut f = std::fs::File::open(file)?;
    let len = f.metadata()?.len();
    let (from, to) = match range {
        Some((a, b)) => (a.min(len), b.unwrap_or(len.saturating_sub(1)).min(len.saturating_sub(1))),
        None => (0, len.saturating_sub(1)),
    };
    let n = if len == 0 { 0 } else { to.saturating_sub(from) + 1 };
    if range.is_some() {
        write!(out, "HTTP/1.1 206 Partial Content\r\nContent-Type: {mime}\r\nContent-Length: {n}\r\nContent-Range: bytes {from}-{to}/{len}\r\nAccept-Ranges: bytes\r\n{DLNA}Connection: close\r\n\r\n")?;
    } else {
        write!(out, "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {len}\r\nAccept-Ranges: bytes\r\n{DLNA}Connection: close\r\n\r\n")?;
    }
    if head_only {
        return Ok(());
    }
    f.seek(SeekFrom::Start(from))?;
    let mut left = n;
    let mut buf = vec![0u8; 64 * 1024];
    while left > 0 {
        let want = left.min(buf.len() as u64) as usize;
        let k = f.read(&mut buf[..want])?;
        if k == 0 {
            break;
        }
        out.write_all(&buf[..k])?;
        left -= k as u64;
    }
    Ok(())
}

/// Relays a remote stream (keeping Range) as plain HTTP.
fn proxy(mut out: TcpStream, url: &str, mime: &str, range: Option<String>, head_only: bool) -> std::io::Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    rt.block_on(async move {
        let client = reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).build().map_err(std::io::Error::other)?;
        let mut req = client.get(url);
        if let Some(r) = &range {
            req = req.header("Range", r.as_str());
        }
        let mut resp = req.send().await.map_err(std::io::Error::other)?;
        let status = resp.status().as_u16();
        let reason = if status == 206 { "Partial Content" } else if status < 300 { "OK" } else { "Error" };
        let mut head = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: {mime}\r\nAccept-Ranges: bytes\r\n{DLNA}Connection: close\r\n");
        if let Some(n) = resp.content_length() {
            head.push_str(&format!("Content-Length: {n}\r\n"));
        }
        if let Some(cr) = resp.headers().get("content-range").and_then(|v| v.to_str().ok()) {
            head.push_str(&format!("Content-Range: {cr}\r\n"));
        }
        head.push_str("\r\n");
        out.write_all(head.as_bytes())?;
        if head_only || status >= 300 {
            return Ok(());
        }
        while let Some(chunk) = resp.chunk().await.map_err(std::io::Error::other)? {
            out.write_all(&chunk)?;
        }
        Ok::<(), std::io::Error>(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types() {
        assert_eq!(guess_mime("https://r1.googlevideo.com/videoplayback?itag=140&mime=audio%2Fmp4&x=1", None), "audio/mp4");
        assert_eq!(guess_mime("https://cdn.example/a/b.ogg", None), "audio/ogg");
        assert_eq!(guess_mime("C:\\Music\\a.flac", None), "audio/flac");
        assert_eq!(guess_mime("x", Some("audio/webm; codecs=\"opus\"")), "audio/webm");
        assert_eq!(ext_for("audio/mp4"), "m4a");
        assert_eq!(ext_for("audio/mpeg"), "mp3");
        assert_eq!(parse_range("bytes=100-"), Some((100, None)));
        assert_eq!(parse_range("bytes=0-99"), Some((0, Some(99))));
        assert_eq!(parse_range("items=0-1"), None);
    }

    #[test]
    fn serves_ranges() {
        let dir = std::env::temp_dir().join(format!("glyph-media-{}", glyph_core::random_token()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("song.mp3");
        std::fs::write(&f, b"0123456789").unwrap();
        let srv = MediaServer::start().unwrap();
        let url = srv.url_for(f.to_str().unwrap(), "audio/mpeg", "127.0.0.1".parse().unwrap()).unwrap();
        assert!(url.ends_with(".mp3"));
        let r = super::super::http::request("GET", &url, &[("Range", "bytes=2-5")], &[], Duration::from_secs(5)).unwrap();
        assert_eq!(r.status, 206);
        assert_eq!(r.body, b"2345");
        assert_eq!(r.header("content-range"), Some("bytes 2-5/10"));
        let r = super::super::http::get(&url, Duration::from_secs(5)).unwrap();
        assert_eq!((r.status, r.body.len()), (200, 10));
        let _ = std::fs::remove_dir_all(dir);
    }
}
