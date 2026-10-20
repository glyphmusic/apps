//! MPD (Music Player Daemon) as an output: Glyph finds the song and MPD on
//! another computer (a Raspberry Pi on the stereo, a server in the living
//! room) plays it. Glyph stays the remote: play / pause, seek, volume, next.
//!
//! MPD fetches the stream itself. Songs that only exist on this computer
//! (downloads, the playback cache) are served to it over a small HTTP file
//! server on the local network, one random path per song.
//!
//! Address format: `host[:port]`, optionally `password@host[:port]`
//! (default port 6600).

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Where MPD runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Address {
    pub host: String,
    pub port: u16,
    pub password: Option<String>,
}

impl Address {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let (password, rest) = match s.rsplit_once('@') {
            Some((p, r)) => (Some(p.to_string()).filter(|p| !p.is_empty()), r),
            None => (None, s),
        };
        let (host, port) = match rest.rsplit_once(':') {
            Some((h, p)) if !h.contains(':') || h.starts_with('[') => (h.trim_matches(['[', ']']).to_string(), p.parse().ok()?),
            _ => (rest.to_string(), 6600),
        };
        (!host.is_empty()).then_some(Self { host, port, password })
    }

    pub fn label(&self) -> String {
        if self.port == 6600 { self.host.clone() } else { format!("{}:{}", self.host, self.port) }
    }
}

/// What MPD reports.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    /// "play" | "pause" | "stop"
    pub state: String,
    pub elapsed: Option<f64>,
    pub duration: Option<f64>,
    /// 0..=100, `None` when MPD has no mixer.
    pub volume: Option<u8>,
}

/// One protocol connection.
pub struct Conn {
    reader: BufReader<TcpStream>,
    stream: TcpStream,
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

impl Conn {
    pub fn open(a: &Address) -> std::io::Result<Self> {
        let addr = (a.host.as_str(), a.port).to_socket_addrs()?.next().ok_or_else(|| std::io::Error::other("unknown host"))?;
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let mut c = Self { reader: BufReader::new(stream.try_clone()?), stream };
        let mut hello = String::new();
        c.reader.read_line(&mut hello)?;
        if !hello.starts_with("OK MPD") {
            return Err(std::io::Error::other("not an MPD server"));
        }
        if let Some(p) = &a.password {
            c.cmd(&format!("password {}", quote(p)))?;
        }
        Ok(c)
    }

    /// Runs one command; its `key: value` lines.
    pub fn cmd(&mut self, line: &str) -> std::io::Result<Vec<(String, String)>> {
        self.stream.write_all(line.as_bytes())?;
        self.stream.write_all(b"\n")?;
        let mut out = Vec::new();
        loop {
            let mut l = String::new();
            if self.reader.read_line(&mut l)? == 0 {
                return Err(std::io::Error::other("MPD closed the connection"));
            }
            let l = l.trim_end();
            if l == "OK" {
                return Ok(out);
            }
            if let Some(e) = l.strip_prefix("ACK ") {
                return Err(std::io::Error::other(format!("MPD: {e}")));
            }
            if let Some((k, v)) = l.split_once(": ") {
                out.push((k.to_string(), v.to_string()));
            }
        }
    }

    pub fn status(&mut self) -> std::io::Result<Status> {
        let kv = self.cmd("status")?;
        let get = |k: &str| kv.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        Ok(Status {
            state: get("state").unwrap_or("stop").to_string(),
            elapsed: get("elapsed").and_then(|v| v.parse().ok()),
            duration: get("duration").and_then(|v| v.parse().ok()),
            volume: get("volume").and_then(|v| v.parse::<i32>().ok()).filter(|v| *v >= 0).map(|v| v.clamp(0, 100) as u8),
        })
    }

    /// Replaces MPD's playlist with `url` and starts it.
    pub fn play_url(&mut self, url: &str, start: bool) -> std::io::Result<()> {
        self.cmd("clear")?;
        self.cmd(&format!("add {}", quote(url)))?;
        if start {
            self.cmd("play 0")?;
        }
        Ok(())
    }
}

// ------------------------------------------------------------ file server

/// Serves local song files to MPD over the network (random path per file).
pub struct FileServer {
    pub port: u16,
    files: Arc<Mutex<std::collections::HashMap<String, std::path::PathBuf>>>,
}

impl FileServer {
    pub fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind(("0.0.0.0", 0))?;
        let port = listener.local_addr()?.port();
        let files: Arc<Mutex<std::collections::HashMap<String, std::path::PathBuf>>> = Default::default();
        let f2 = files.clone();
        std::thread::Builder::new().name("glyph-mpd-files".into()).spawn(move || {
            for s in listener.incoming().flatten() {
                let files = f2.clone();
                let _ = std::thread::Builder::new().name("glyph-mpd-file".into()).spawn(move || {
                    if let Err(e) = serve(s, &files) {
                        tracing::debug!("MPD file request: {e}");
                    }
                });
            }
        })?;
        Ok(Self { port, files })
    }

    /// URL MPD can fetch `path` from (this computer's LAN address).
    pub fn url_for(&self, path: &std::path::Path, mpd: &Address) -> Option<String> {
        let id = glyph_core::random_token();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("m4a").to_string();
        let mut f = self.files.lock().unwrap();
        if f.len() > 50 {
            f.clear();
        }
        f.insert(id.clone(), path.to_path_buf());
        let ip = local_ip_towards(mpd)?;
        Some(format!("http://{ip}:{}/{id}.{ext}", self.port))
    }
}

/// The address of this computer on the route to MPD.
fn local_ip_towards(mpd: &Address) -> Option<String> {
    let target = (mpd.host.as_str(), mpd.port).to_socket_addrs().ok()?.next()?;
    let sock = std::net::UdpSocket::bind(if target.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" }).ok()?;
    sock.connect(target).ok()?;
    let ip = sock.local_addr().ok()?.ip();
    Some(if ip.is_ipv6() { format!("[{ip}]") } else { ip.to_string() })
}

fn serve(s: TcpStream, files: &Mutex<std::collections::HashMap<String, std::path::PathBuf>>) -> std::io::Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut r = BufReader::new(s.try_clone()?);
    let mut first = String::new();
    r.read_line(&mut first)?;
    let path = first.split_whitespace().nth(1).unwrap_or("/").trim_start_matches('/').to_string();
    let mut range: Option<u64> = None;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some(v) = h.to_ascii_lowercase().strip_prefix("range: bytes=") {
            range = v.trim().split('-').next().and_then(|n| n.parse().ok());
        }
    }
    let id = path.split('.').next().unwrap_or("");
    let file = files.lock().unwrap().get(id).cloned();
    let mut out = s;
    let Some(file) = file else {
        return out.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    };
    let mut f = std::fs::File::open(&file)?;
    let len = f.metadata()?.len();
    let mime = glyph_core::downloads::mime_of(&file);
    let from = range.unwrap_or(0).min(len);
    f.seek(SeekFrom::Start(from))?;
    if range.is_some() {
        write!(out, "HTTP/1.1 206 Partial Content\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nContent-Range: bytes {from}-{}/{len}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n", len - from, len.saturating_sub(1))?;
    } else {
        write!(out, "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {len}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n")?;
    }
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
    }
    Ok(())
}

// ------------------------------------------------------------ the output

/// Commands for the MPD worker thread.
pub enum Cmd {
    Play { url: String, start: bool },
    Pause(bool),
    Seek(f64),
    Volume(u8),
    Stop,
}

/// The MPD output: a worker thread holding the connection, and the last
/// status it saw (polled twice a second).
pub struct Output {
    pub address: Address,
    tx: flume::Sender<Cmd>,
    pub status: Arc<Mutex<Option<Status>>>,
    pub error: Arc<Mutex<Option<String>>>,
    files: Option<FileServer>,
}

impl Output {
    pub fn start(address: Address) -> Self {
        let (tx, rx) = flume::unbounded::<Cmd>();
        let status: Arc<Mutex<Option<Status>>> = Default::default();
        let error: Arc<Mutex<Option<String>>> = Default::default();
        let (st, err, a) = (status.clone(), error.clone(), address.clone());
        let _ = std::thread::Builder::new().name("glyph-mpd".into()).spawn(move || {
            let mut conn: Option<Conn> = None;
            loop {
                let cmd = match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(c) => Some(c),
                    Err(flume::RecvTimeoutError::Timeout) => None,
                    Err(flume::RecvTimeoutError::Disconnected) => break,
                };
                if conn.is_none() {
                    match Conn::open(&a) {
                        Ok(c) => {
                            conn = Some(c);
                            *err.lock().unwrap() = None;
                        }
                        Err(e) => {
                            *err.lock().unwrap() = Some(e.to_string());
                            *st.lock().unwrap() = None;
                            std::thread::sleep(Duration::from_secs(2));
                            continue;
                        }
                    }
                }
                let c = conn.as_mut().unwrap();
                let r = (|| -> std::io::Result<()> {
                    match cmd {
                        Some(Cmd::Play { url, start }) => c.play_url(&url, start)?,
                        Some(Cmd::Pause(p)) => {
                            c.cmd(&format!("pause {}", u8::from(p)))?;
                        }
                        Some(Cmd::Seek(s)) => {
                            c.cmd(&format!("seekcur {s:.1}"))?;
                        }
                        Some(Cmd::Volume(v)) => {
                            c.cmd(&format!("setvol {v}"))?;
                        }
                        Some(Cmd::Stop) => {
                            c.cmd("stop")?;
                        }
                        None => {}
                    }
                    let s = c.status()?;
                    *st.lock().unwrap() = Some(s);
                    Ok(())
                })();
                if let Err(e) = r {
                    *err.lock().unwrap() = Some(e.to_string());
                    conn = None;
                }
            }
        });
        Self { address, tx, status, error, files: FileServer::start().ok() }
    }

    pub fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
    }

    /// A URL MPD can open for a playback source URL or local file path.
    pub fn url_for(&self, src: &str) -> Option<String> {
        if src.starts_with("http://") || src.starts_with("https://") {
            return Some(src.to_string());
        }
        let p = std::path::Path::new(src);
        p.is_file().then(|| self.files.as_ref()?.url_for(p, &self.address)).flatten()
    }

    pub fn status(&self) -> Option<Status> {
        self.status.lock().unwrap().clone()
    }

    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(Address::parse("pi.local"), Some(Address { host: "pi.local".into(), port: 6600, password: None }));
        assert_eq!(Address::parse("pw@10.0.0.2:6601"), Some(Address { host: "10.0.0.2".into(), port: 6601, password: Some("pw".into()) }));
        assert_eq!(Address::parse("[::1]:6600").map(|a| a.host), Some("::1".into()));
        assert_eq!(Address::parse("  "), None);
        assert_eq!(quote("a \"b\" \\"), "\"a \\\"b\\\" \\\\\"");
    }

    #[test]
    fn talks_the_protocol() {
        let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let mut w = s.try_clone().unwrap();
            let mut r = BufReader::new(s);
            w.write_all(b"OK MPD 0.23.5\n").unwrap();
            let mut line = String::new();
            while r.read_line(&mut line).unwrap() > 0 {
                let reply: &[u8] = match line.trim() {
                    "status" => b"volume: 40\nstate: play\nelapsed: 12.5\nduration: 200.0\nOK\n",
                    l if l.starts_with("add ") => b"OK\n",
                    "bogus" => b"ACK [5@0] {} unknown command \"bogus\"\n",
                    _ => b"OK\n",
                };
                w.write_all(reply).unwrap();
                line.clear();
            }
        });
        let mut c = Conn::open(&Address { host: "127.0.0.1".into(), port, password: None }).unwrap();
        c.play_url("https://example.com/a.mp3", true).unwrap();
        let s = c.status().unwrap();
        assert_eq!((s.state.as_str(), s.elapsed, s.duration, s.volume), ("play", Some(12.5), Some(200.0), Some(40)));
        assert!(c.cmd("bogus").is_err());
    }
}
