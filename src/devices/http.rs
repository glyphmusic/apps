//! Tiny blocking HTTP/1.1 client for devices on the local network (UPnP
//! descriptions and SOAP control): plain `http://`, one request per
//! connection.

use std::io::{Read, Write};
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub struct Url {
    pub host: String,
    pub port: u16,
    /// Starts with `/`.
    pub path: String,
}

impl Url {
    pub fn parse(u: &str) -> Option<Self> {
        let rest = u.strip_prefix("http://")?;
        let (hp, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let (host, port) = if let Some(h) = hp.strip_prefix('[') {
            let (h, p) = h.split_once(']')?;
            (h.to_string(), p.strip_prefix(':').map(|p| p.parse().ok()).unwrap_or(Some(80))?)
        } else {
            match hp.rsplit_once(':') {
                Some((h, p)) => (h.to_string(), p.parse().ok()?),
                None => (hp.to_string(), 80),
            }
        };
        (!host.is_empty()).then(|| Self { host, port, path: path.to_string() })
    }

    /// `http://host:port`
    pub fn origin(&self) -> String {
        if self.host.contains(':') { format!("http://[{}]:{}", self.host, self.port) } else { format!("http://{}:{}", self.host, self.port) }
    }

    pub fn ip(&self) -> Option<IpAddr> {
        self.host.parse().ok().or_else(|| (self.host.as_str(), self.port).to_socket_addrs().ok()?.next().map(|a| a.ip()))
    }
}

/// `rel` (absolute URL, absolute path or relative path) against `base`.
pub fn join(base: &str, rel: &str) -> String {
    let rel = rel.trim();
    if rel.starts_with("http://") || rel.starts_with("https://") {
        return rel.to_string();
    }
    let Some(b) = Url::parse(base) else { return rel.to_string() };
    if rel.starts_with('/') {
        return format!("{}{rel}", b.origin());
    }
    let dir = b.path.split('?').next().unwrap_or("/");
    let dir = &dir[..dir.rfind('/').map_or(0, |i| i + 1)];
    format!("{}{}{rel}", b.origin(), if dir.is_empty() { "/" } else { dir })
}

#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Decodes a `Transfer-Encoding: chunked` body; `None` while incomplete.
pub fn decode_chunked(mut b: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let nl = b.windows(2).position(|w| w == b"\r\n")?;
        let size_s = std::str::from_utf8(&b[..nl]).ok()?;
        let size = usize::from_str_radix(size_s.split(';').next()?.trim(), 16).ok()?;
        b = &b[nl + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(b.get(..size)?);
        b = b.get(size + 2..)?;
    }
}

/// Splits a raw response; `None` while it is incomplete.
pub fn parse_response(raw: &[u8], eof: bool) -> Option<Response> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&raw[..end]);
    let mut lines = head.split("\r\n");
    let status = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
    let rest = &raw[end + 4..];
    let get = |n: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.clone());
    let body = if get("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked")) {
        match decode_chunked(rest) {
            Some(b) => b,
            None if eof => rest.to_vec(),
            None => return None,
        }
    } else if let Some(len) = get("content-length").and_then(|v| v.parse::<usize>().ok()) {
        if rest.len() < len && !eof {
            return None;
        }
        rest[..len.min(rest.len())].to_vec()
    } else if eof || status == 204 || status == 304 {
        rest.to_vec()
    } else {
        return None;
    };
    Some(Response { status, headers, body })
}

/// One request; the whole response.
pub fn request(method: &str, url: &str, headers: &[(&str, &str)], body: &[u8], timeout: Duration) -> std::io::Result<Response> {
    let u = Url::parse(url).ok_or_else(|| std::io::Error::other(format!("bad URL {url}")))?;
    let addr = (u.host.as_str(), u.port).to_socket_addrs()?.next().ok_or_else(|| std::io::Error::other("unknown host"))?;
    let mut s = TcpStream::connect_timeout(&addr, timeout.min(Duration::from_secs(4)))?;
    s.set_read_timeout(Some(Duration::from_millis(500)))?;
    s.set_write_timeout(Some(timeout))?;
    let host = if u.host.contains(':') { format!("[{}]:{}", u.host, u.port) } else { format!("{}:{}", u.host, u.port) };
    let mut req = format!("{method} {} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nUser-Agent: Glyph/{}\r\n", u.path, env!("CARGO_PKG_VERSION"));
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if !body.is_empty() || method == "POST" {
        req.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes())?;
    s.write_all(body)?;
    let started = Instant::now();
    let mut raw = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        match s.read(&mut buf) {
            Ok(0) => return parse_response(&raw, true).ok_or_else(|| std::io::Error::other("bad HTTP response")),
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                if raw.len() > 8 << 20 {
                    return Err(std::io::Error::other("response too large"));
                }
                if let Some(r) = parse_response(&raw, false) {
                    return Ok(r);
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(e),
        }
        if started.elapsed() > timeout {
            return parse_response(&raw, true).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::TimedOut, "device did not answer"));
        }
    }
}

pub fn get(url: &str, timeout: Duration) -> std::io::Result<Response> {
    request("GET", url, &[], &[], timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        let u = Url::parse("http://192.168.1.30:1400/xml/device_description.xml").unwrap();
        assert_eq!((u.host.as_str(), u.port, u.path.as_str()), ("192.168.1.30", 1400, "/xml/device_description.xml"));
        assert_eq!(Url::parse("http://tv.local").unwrap().path, "/");
        assert_eq!(Url::parse("http://[fe80::1]:8080/a").unwrap().host, "fe80::1");
        assert_eq!(Url::parse("https://x/"), None);
        assert_eq!(join("http://10.0.0.5:52235/dmr/desc.xml", "/upnp/control/AVTransport1"), "http://10.0.0.5:52235/upnp/control/AVTransport1");
        assert_eq!(join("http://10.0.0.5:52235/dmr/desc.xml", "control/AVT"), "http://10.0.0.5:52235/dmr/control/AVT");
        assert_eq!(join("http://10.0.0.5:52235", "AVT"), "http://10.0.0.5:52235/AVT");
        assert_eq!(join("http://a:1/x", "http://b:2/y"), "http://b:2/y");
    }

    #[test]
    fn responses() {
        let r = parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nX: y\r\n\r\nhello", false).unwrap();
        assert_eq!((r.status, r.body.as_slice(), r.header("x")), (200, &b"hello"[..], Some("y")));
        assert!(parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nhello", false).is_none());
        let c = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6;x=y\r\n world\r\n0\r\n\r\n";
        assert_eq!(parse_response(c, false).unwrap().text(), "hello world");
        assert!(parse_response(b"HTTP/1.1 200 OK\r\n\r\npartial", false).is_none());
        assert_eq!(parse_response(b"HTTP/1.1 500 Internal\r\n\r\nall", true).unwrap().status, 500);
    }

    #[test]
    fn round_trip() {
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut buf = [0u8; 1024];
            let n = s.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            assert!(req.starts_with("POST /ctl HTTP/1.1\r\n"));
            assert!(req.contains("SOAPACTION: x"));
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok").unwrap();
            // Keeps the connection open: the client must stop at Content-Length.
            std::thread::sleep(Duration::from_secs(2));
        });
        let r = request("POST", &format!("http://127.0.0.1:{port}/ctl"), &[("SOAPACTION", "x")], b"body", Duration::from_secs(5)).unwrap();
        assert_eq!(r.text(), "ok");
    }
}
