//! Loopback HTTP server (127.0.0.1 only, random token in every path) that
//! serves the player page and bridges commands/events for the browser
//! transport:
//!
//! * `GET  /<token>/player.html` the page
//! * `GET  /<token>/poll`        long-poll: JSON array of JS strings to `eval`
//! * `POST /<token>/event`       one JSON event message from the page

use super::{parse_message, EventSink, PLAYER_HTML};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const POLL_WAIT: Duration = Duration::from_secs(15);

pub struct LocalServer {
    port: u16,
    token: String,
    queue: Mutex<VecDeque<String>>,
    cv: Condvar,
    last_poll: Mutex<Option<Instant>>,
    sink: EventSink,
}

fn random_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for i in 0..2u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        );
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

impl LocalServer {
    pub fn start(sink: EventSink) -> io::Result<Arc<Self>> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let srv = Arc::new(Self {
            port,
            token: random_token(),
            queue: Mutex::new(VecDeque::new()),
            cv: Condvar::new(),
            last_poll: Mutex::new(None),
            sink,
        });
        let weak = Arc::downgrade(&srv);
        std::thread::Builder::new().name("glyph-embed-http".into()).spawn(move || {
            for stream in listener.incoming() {
                let Some(srv) = weak.upgrade() else { break };
                let Ok(stream) = stream else { continue };
                std::thread::spawn(move || {
                    let _ = srv.handle(stream);
                });
            }
        })?;
        Ok(srv)
    }

    pub fn page_url(&self) -> String {
        format!("http://127.0.0.1:{}/{}/player.html", self.port, self.token)
    }

    pub fn push_script(&self, js: String) {
        self.queue.lock().unwrap().push_back(js);
        self.cv.notify_all();
    }

    pub fn clear_queue(&self) {
        self.queue.lock().unwrap().clear();
    }

    /// Time since the page last polled (browser transport liveness).
    pub fn poll_age(&self) -> Option<Duration> {
        self.last_poll.lock().unwrap().map(|t| t.elapsed())
    }

    fn handle(&self, mut s: TcpStream) -> io::Result<()> {
        s.set_read_timeout(Some(Duration::from_secs(10)))?;
        let (method, path, body) = read_request(&mut s)?;
        let Some(rest) = path.strip_prefix('/').and_then(|p| p.strip_prefix(self.token.as_str())) else {
            return respond(&mut s, 404, "text/plain", b"not found");
        };
        let route = rest.split('?').next().unwrap_or(rest);
        match (method.as_str(), route) {
            ("GET", "/player.html") => respond(&mut s, 200, "text/html; charset=utf-8", PLAYER_HTML.as_bytes()),
            ("GET", "/poll") => {
                *self.last_poll.lock().unwrap() = Some(Instant::now());
                let scripts = self.wait_scripts();
                *self.last_poll.lock().unwrap() = Some(Instant::now());
                let json = serde_json::to_vec(&scripts).unwrap_or_else(|_| b"[]".to_vec());
                respond(&mut s, 200, "application/json", &json)
            }
            ("POST", "/event") => {
                if let Some((session, ev)) = parse_message(&String::from_utf8_lossy(&body)) {
                    (self.sink)(session, ev);
                }
                respond(&mut s, 204, "text/plain", b"")
            }
            _ => respond(&mut s, 404, "text/plain", b"not found"),
        }
    }

    fn wait_scripts(&self) -> Vec<String> {
        let deadline = Instant::now() + POLL_WAIT;
        let mut q = self.queue.lock().unwrap();
        loop {
            if !q.is_empty() {
                return q.drain(..).collect();
            }
            let now = Instant::now();
            if now >= deadline {
                return Vec::new();
            }
            q = self.cv.wait_timeout(q, deadline - now).unwrap().0;
        }
    }
}

fn read_request(s: &mut TcpStream) -> io::Result<(String, String, Vec<u8>)> {
    let mut data = Vec::new();
    let mut tmp = [0u8; 2048];
    let head_end = loop {
        if let Some(i) = find(&data, b"\r\n\r\n") {
            break i;
        }
        if data.len() > 64 * 1024 {
            return Err(io::Error::other("header too large"));
        }
        let n = s.read(&mut tmp)?;
        if n == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        data.extend_from_slice(&tmp[..n]);
    };
    let head = String::from_utf8_lossy(&data[..head_end]).to_string();
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or("").split_whitespace();
    let method = first.next().unwrap_or("").to_string();
    let path = first.next().unwrap_or("").to_string();
    let len: usize = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse().ok())
        .unwrap_or(0)
        .min(1 << 20);
    let mut body = data[head_end + 4..].to_vec();
    while body.len() < len {
        let n = s.read(&mut tmp)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(len);
    Ok((method, path, body))
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

fn respond(s: &mut TcpStream, code: u16, ctype: &str, body: &[u8]) -> io::Result<()> {
    let reason = match code {
        200 => "OK",
        204 => "No Content",
        _ => "Not Found",
    };
    write!(
        s,
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: strict-origin-when-cross-origin\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    s.write_all(body)?;
    s.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::{EmbedEvent, EmbedPlayState};

    fn http(port: u16, req: &str) -> String {
        let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
        c.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        c.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn serves_page_bridges_events_and_scripts() {
        let (tx, rx) = flume::unbounded();
        let sink: EventSink = Arc::new(move |s, e| {
            let _ = tx.send((s, e));
        });
        let srv = LocalServer::start(sink).unwrap();
        let url = srv.page_url();
        let path = url.split_once(&format!(":{}", srv.port)).unwrap().1.to_string();

        let page = http(srv.port, &format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n"));
        assert!(page.starts_with("HTTP/1.1 200"));
        assert!(page.contains("window.glyph"));

        let bad = http(srv.port, "GET /nope/player.html HTTP/1.1\r\n\r\n");
        assert!(bad.starts_with("HTTP/1.1 404"));

        let ev_path = path.replace("player.html", "event");
        let body = r#"{"t":"state","v":"playing","s":9}"#;
        let r = http(
            srv.port,
            &format!("POST {ev_path} HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}", body.len()),
        );
        assert!(r.starts_with("HTTP/1.1 204"));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), (9, EmbedEvent::State(EmbedPlayState::Playing)));

        srv.push_script("window.glyph.play();".into());
        let poll_path = path.replace("player.html", "poll");
        let r = http(srv.port, &format!("GET {poll_path} HTTP/1.1\r\n\r\n"));
        assert!(r.ends_with(r#"["window.glyph.play();"]"#), "{r}");
        assert!(srv.poll_age().is_some());
    }
}
