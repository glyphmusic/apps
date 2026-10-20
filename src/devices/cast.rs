//! Google Cast (Chromecast, Nest speakers, TVs with Chromecast built-in):
//! TLS to port 8009, length-prefixed protobuf `CastMessage`s carrying JSON.
//! Glyph launches the Default Media Receiver and hands it the song URL.

use super::{Media, Session};
use crate::mpd::Status;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

const NS_CONNECTION: &str = "urn:x-cast:com.google.cast.tp.connection";
const NS_HEARTBEAT: &str = "urn:x-cast:com.google.cast.tp.heartbeat";
const NS_RECEIVER: &str = "urn:x-cast:com.google.cast.receiver";
const NS_MEDIA: &str = "urn:x-cast:com.google.cast.media";
/// Default Media Receiver.
const APP_ID: &str = "CC1AD845";
const SENDER: &str = "sender-glyph";
const RECEIVER: &str = "receiver-0";

// ------------------------------------------------------------ wire format

/// A `CastMessage` with a string payload.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub source: String,
    pub destination: String,
    pub namespace: String,
    pub payload: String,
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn put_str(out: &mut Vec<u8>, field: u32, s: &str) {
    put_varint(out, ((field << 3) | 2) as u64);
    put_varint(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

fn get_varint(b: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *b.get(*pos)?;
        *pos += 1;
        v |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

impl Message {
    pub fn new(destination: &str, namespace: &str, payload: &Value) -> Self {
        Self { source: SENDER.into(), destination: destination.into(), namespace: namespace.into(), payload: payload.to_string() }
    }

    /// Protobuf encoding (without the length prefix).
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(64 + self.payload.len());
        o.extend_from_slice(&[0x08, 0x00]); // protocol_version = CASTV2_1_0
        put_str(&mut o, 2, &self.source);
        put_str(&mut o, 3, &self.destination);
        put_str(&mut o, 4, &self.namespace);
        o.extend_from_slice(&[0x28, 0x00]); // payload_type = STRING
        put_str(&mut o, 6, &self.payload);
        o
    }

    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut m = Message { source: String::new(), destination: String::new(), namespace: String::new(), payload: String::new() };
        let mut pos = 0;
        while pos < b.len() {
            let key = get_varint(b, &mut pos)?;
            match key & 7 {
                0 => {
                    get_varint(b, &mut pos)?;
                }
                2 => {
                    let len = get_varint(b, &mut pos)? as usize;
                    let v = b.get(pos..pos.checked_add(len)?)?;
                    pos += len;
                    let s = || String::from_utf8_lossy(v).into_owned();
                    match key >> 3 {
                        2 => m.source = s(),
                        3 => m.destination = s(),
                        4 => m.namespace = s(),
                        6 => m.payload = s(),
                        _ => {}
                    }
                }
                5 => pos += 4,
                1 => pos += 8,
                _ => return None,
            }
        }
        Some(m)
    }
}

/// Complete frames at the front of `buf` (removed from it).
pub fn take_frames(buf: &mut Vec<u8>) -> Vec<Message> {
    let mut out = Vec::new();
    loop {
        if buf.len() < 4 {
            return out;
        }
        let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        if len > 1 << 20 {
            // Garbage: drop everything rather than wait for a megabyte.
            buf.clear();
            return out;
        }
        if buf.len() < 4 + len {
            return out;
        }
        if let Some(m) = Message::decode(&buf[4..4 + len]) {
            out.push(m);
        }
        buf.drain(..4 + len);
    }
}

// ------------------------------------------------------------ TLS

/// Cast devices present self-signed certificates; the connection is only
/// encrypted, not authenticated (as in every open Cast sender).
#[derive(Debug)]
struct AnyCert(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for AnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

type Tls = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

fn tls_connect(addr: SocketAddr) -> std::io::Result<Tls> {
    let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(4))?;
    tcp.set_nodelay(true)?;
    tcp.set_read_timeout(Some(Duration::from_millis(200)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(5)))?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(std::io::Error::other)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyCert(provider)))
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(addr.ip().to_string()).map_err(std::io::Error::other)?;
    let conn = rustls::ClientConnection::new(Arc::new(cfg), name).map_err(std::io::Error::other)?;
    let mut tls = rustls::StreamOwned::new(conn, tcp);
    // Finish the handshake now, so a wrong port fails here.
    let started = Instant::now();
    while tls.conn.is_handshaking() {
        match tls.conn.complete_io(&mut tls.sock) {
            Ok(_) => {}
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) && started.elapsed() < Duration::from_secs(5) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(tls)
}

// ------------------------------------------------------------ session

/// A connection to one Cast device running the Default Media Receiver.
pub struct Client {
    tls: Tls,
    buf: Vec<u8>,
    request: u64,
    /// The receiver app's transport id and session id.
    app: Option<(String, String)>,
    media_session: Option<i64>,
    last_ping: Instant,
    last_receiver_poll: Instant,
    volume: Option<u8>,
    duration: Option<f64>,
    /// Last media state seen (kept between polls without news).
    state: String,
    elapsed: Option<f64>,
    elapsed_at: Instant,
    loaded: bool,
}

impl Client {
    pub fn connect(d: &super::Device) -> std::io::Result<Self> {
        let tls = tls_connect(SocketAddr::new(d.addr, d.port))?;
        let mut c = Self {
            tls,
            buf: Vec::new(),
            request: 1,
            app: None,
            media_session: None,
            last_ping: Instant::now(),
            last_receiver_poll: Instant::now() - Duration::from_secs(60),
            volume: None,
            duration: None,
            state: "stop".into(),
            elapsed: None,
            elapsed_at: Instant::now(),
            loaded: false,
        };
        c.send(RECEIVER, NS_CONNECTION, json!({"type": "CONNECT", "userAgent": "Glyph"}))?;
        Ok(c)
    }

    fn send(&mut self, dest: &str, ns: &str, payload: Value) -> std::io::Result<()> {
        let body = Message::new(dest, ns, &payload).encode();
        let mut frame = (body.len() as u32).to_be_bytes().to_vec();
        frame.extend_from_slice(&body);
        self.tls.write_all(&frame)?;
        self.tls.flush()
    }

    fn next_id(&mut self) -> u64 {
        self.request += 1;
        self.request
    }

    /// Reads what arrived for up to `wait`, answering pings and keeping
    /// state; returns the JSON payloads.
    fn pump(&mut self, wait: Duration) -> std::io::Result<Vec<Value>> {
        let until = Instant::now() + wait;
        let mut out = Vec::new();
        let mut tmp = [0u8; 16 * 1024];
        loop {
            match self.tls.read(&mut tmp) {
                Ok(0) => return Err(std::io::Error::other("Cast device closed the connection")),
                Ok(n) => self.buf.extend_from_slice(&tmp[..n]),
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(e) => return Err(e),
            }
            for m in take_frames(&mut self.buf) {
                let Ok(v) = serde_json::from_str::<Value>(&m.payload) else { continue };
                let kind = v["type"].as_str().map(str::to_string);
                match kind.as_deref() {
                    Some("PING") if m.namespace == NS_HEARTBEAT => {
                        let src = m.source.clone();
                        self.send(&src, NS_HEARTBEAT, json!({"type": "PONG"}))?;
                    }
                    Some("CLOSE") if self.app.as_ref().is_some_and(|(t, _)| *t == m.source) => {
                        // Another sender took over (or the app quit).
                        self.app = None;
                        self.media_session = None;
                        self.state = "closed".into();
                    }
                    _ => {
                        self.observe(&v);
                        out.push(v);
                    }
                }
            }
            if Instant::now() >= until {
                return Ok(out);
            }
        }
    }

    /// Waits for a payload `want` says is the answer.
    fn wait_for(&mut self, wait: Duration, want: impl Fn(&Value) -> bool) -> std::io::Result<Option<Value>> {
        let until = Instant::now() + wait;
        while Instant::now() < until {
            for v in self.pump(Duration::from_millis(200))? {
                if matches!(v["type"].as_str(), Some("LOAD_FAILED" | "LOAD_CANCELLED" | "INVALID_REQUEST" | "LAUNCH_ERROR")) {
                    let why = v["reason"].as_str().unwrap_or(v["type"].as_str().unwrap_or_default()).to_string();
                    return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("Cast: {why}")));
                }
                if want(&v) {
                    return Ok(Some(v));
                }
            }
        }
        Ok(None)
    }

    fn observe(&mut self, v: &Value) {
        match v["type"].as_str() {
            Some("RECEIVER_STATUS") => {
                let st = &v["status"];
                if let Some(l) = st["volume"]["level"].as_f64() {
                    let muted = st["volume"]["muted"].as_bool().unwrap_or(false);
                    self.volume = Some(if muted { 0 } else { (l * 100.0).round().clamp(0.0, 100.0) as u8 });
                }
                let app = st["applications"].as_array().and_then(|a| a.iter().find(|x| x["appId"] == APP_ID));
                match app {
                    Some(a) => {
                        if let (Some(t), Some(s)) = (a["transportId"].as_str(), a["sessionId"].as_str()) {
                            if self.app.as_ref().map(|(x, _)| x.as_str()) != Some(t) {
                                self.media_session = None;
                            }
                            self.app = Some((t.to_string(), s.to_string()));
                        }
                    }
                    None if st["applications"].is_array() || st.get("applications").is_none() => {
                        if self.app.is_some() {
                            self.app = None;
                            self.media_session = None;
                            self.state = "closed".into();
                        }
                    }
                    None => {}
                }
            }
            Some("MEDIA_STATUS") => {
                let Some(s) = v["status"].as_array().and_then(|a| a.first()) else {
                    // No media session anymore.
                    // (After "finished" the session ends too; keep that.)
                    if self.media_session.is_some() {
                        self.media_session = None;
                        if self.state != "stop" {
                            self.state = "closed".into();
                        }
                    }
                    return;
                };
                if let Some(id) = s["mediaSessionId"].as_i64() {
                    self.media_session = Some(id);
                }
                if let Some(d) = s["media"]["duration"].as_f64() {
                    self.duration = Some(d);
                }
                if let Some(t) = s["currentTime"].as_f64() {
                    self.elapsed = Some(t);
                    self.elapsed_at = Instant::now();
                }
                self.state = match s["playerState"].as_str() {
                    Some("PLAYING") => "play",
                    Some("BUFFERING") => "load",
                    Some("PAUSED") => "pause",
                    Some("IDLE") => match s["idleReason"].as_str() {
                        Some("FINISHED") => "stop",
                        Some("ERROR") => "error",
                        // CANCELLED / INTERRUPTED: stopped elsewhere or replaced.
                        Some(_) => "closed",
                        None if self.loaded => "load",
                        None => "closed",
                    },
                    _ => self.state.as_str(),
                }
                .to_string();
                if self.state != "load" {
                    self.loaded = false;
                }
            }
            _ => {}
        }
    }

    /// Launches the Default Media Receiver (or joins it) and connects to it.
    fn ensure_app(&mut self) -> std::io::Result<String> {
        if self.app.is_none() {
            let id = self.next_id();
            self.send(RECEIVER, NS_RECEIVER, json!({"type": "GET_STATUS", "requestId": id}))?;
            self.wait_for(Duration::from_secs(3), |v| v["type"] == "RECEIVER_STATUS")?;
        }
        if self.app.is_none() {
            let id = self.next_id();
            self.send(RECEIVER, NS_RECEIVER, json!({"type": "LAUNCH", "appId": APP_ID, "requestId": id}))?;
            let until = Instant::now() + Duration::from_secs(15);
            while self.app.is_none() && Instant::now() < until {
                self.wait_for(Duration::from_secs(1), |v| v["type"] == "RECEIVER_STATUS")?;
            }
        }
        let (transport, _) = self.app.clone().ok_or_else(|| std::io::Error::other("Cast: the receiver app did not start"))?;
        self.send(&transport, NS_CONNECTION, json!({"type": "CONNECT", "userAgent": "Glyph"}))?;
        Ok(transport)
    }

    fn media_cmd(&mut self, kind: &str, extra: Value) -> std::io::Result<()> {
        let (Some((transport, _)), Some(ms)) = (self.app.clone(), self.media_session) else { return Ok(()) };
        let id = self.next_id();
        let mut p = json!({"type": kind, "mediaSessionId": ms, "requestId": id});
        if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        self.send(&transport, NS_MEDIA, p)
    }
}

/// The LOAD request for `m`.
pub fn load_request(m: &Media, session_id: &str, request_id: u64, start: bool) -> Value {
    let mut meta = json!({"metadataType": 3, "title": m.title, "artist": m.artist});
    if let Some(a) = &m.album {
        meta["albumName"] = json!(a);
    }
    if let Some(a) = &m.art {
        meta["images"] = json!([{ "url": a }]);
    }
    let stream_type = if m.duration.is_some() { "BUFFERED" } else { "LIVE" };
    let mut media = json!({
        "contentId": m.url,
        "contentUrl": m.url,
        "contentType": m.mime,
        "streamType": stream_type,
        "metadata": meta,
    });
    if let Some(d) = m.duration {
        media["duration"] = json!(d);
    }
    json!({
        "type": "LOAD",
        "requestId": request_id,
        "sessionId": session_id,
        "media": media,
        "autoplay": start,
        "currentTime": m.start_at.max(0.0),
    })
}

impl Session for Client {
    fn load(&mut self, m: &Media, start: bool) -> std::io::Result<()> {
        let transport = self.ensure_app()?;
        let session = self.app.as_ref().map(|(_, s)| s.clone()).unwrap_or_default();
        let id = self.next_id();
        self.media_session = None;
        self.duration = m.duration;
        self.loaded = true;
        self.state = "load".into();
        self.send(&transport, NS_MEDIA, load_request(m, &session, id, start))?;
        if let Err(e) = self.wait_for(Duration::from_secs(8), |v| v["type"] == "MEDIA_STATUS" && v["requestId"].as_u64() == Some(id)) {
            self.state = "error".into();
            self.loaded = false;
            return Err(e);
        }
        Ok(())
    }

    fn pause(&mut self, pause: bool) -> std::io::Result<()> {
        self.media_cmd(if pause { "PAUSE" } else { "PLAY" }, json!({}))
    }

    fn seek(&mut self, secs: f64) -> std::io::Result<()> {
        self.media_cmd("SEEK", json!({"currentTime": secs.max(0.0)}))
    }

    fn volume(&mut self, v: u8) -> std::io::Result<()> {
        let id = self.next_id();
        self.volume = Some(v);
        self.send(RECEIVER, NS_RECEIVER, json!({"type": "SET_VOLUME", "volume": {"level": v as f64 / 100.0}, "requestId": id}))
    }

    fn stop(&mut self) -> std::io::Result<()> {
        // Quits the receiver app: the TV / speaker goes back to idle.
        if let Some((_, session)) = self.app.take() {
            let id = self.next_id();
            self.send(RECEIVER, NS_RECEIVER, json!({"type": "STOP", "sessionId": session, "requestId": id}))?;
        }
        self.media_session = None;
        Ok(())
    }

    fn status(&mut self) -> std::io::Result<Status> {
        if self.last_ping.elapsed() > Duration::from_secs(5) {
            self.last_ping = Instant::now();
            self.send(RECEIVER, NS_HEARTBEAT, json!({"type": "PING"}))?;
        }
        if self.last_receiver_poll.elapsed() > Duration::from_secs(3) {
            self.last_receiver_poll = Instant::now();
            let id = self.next_id();
            self.send(RECEIVER, NS_RECEIVER, json!({"type": "GET_STATUS", "requestId": id}))?;
        }
        if let Some((transport, _)) = self.app.clone() {
            if self.media_session.is_some() {
                let id = self.next_id();
                self.send(&transport, NS_MEDIA, json!({"type": "GET_STATUS", "requestId": id}))?;
            }
        }
        self.pump(Duration::from_millis(250))?;
        // Between status messages the position moves on by itself.
        let elapsed = self.elapsed.map(|e| if self.state == "play" { e + self.elapsed_at.elapsed().as_secs_f64() } else { e });
        Ok(Status { state: self.state.clone(), elapsed, duration: self.duration, volume: self.volume })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protobuf_round_trip() {
        let m = Message::new("receiver-0", NS_RECEIVER, &json!({"type": "GET_STATUS", "requestId": 1}));
        let b = m.encode();
        assert_eq!(&b[..2], &[0x08, 0x00]);
        assert_eq!(b[2], 0x12); // field 2, length-delimited
        assert_eq!(Message::decode(&b), Some(m.clone()));
        // Long payloads need multi-byte lengths.
        let big = Message::new("x", NS_MEDIA, &json!({"t": "a".repeat(300)}));
        assert_eq!(Message::decode(&big.encode()).unwrap().payload, big.payload);
    }

    #[test]
    fn frames() {
        let a = Message::new("r", NS_HEARTBEAT, &json!({"type": "PING"})).encode();
        let mut buf = (a.len() as u32).to_be_bytes().to_vec();
        buf.extend_from_slice(&a);
        buf.extend_from_slice(&[0, 0, 0, 50, 1, 2]); // a second, incomplete frame
        let got = take_frames(&mut buf);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].namespace, NS_HEARTBEAT);
        assert_eq!(buf.len(), 6);
        assert!(Message::decode(&[0x0F]).is_none());
    }

    #[test]
    fn load_message() {
        let m = Media { url: "http://10.0.0.2:4000/a.m4a".into(), mime: "audio/mp4".into(), title: "T".into(), artist: "A".into(), album: Some("B".into()), art: Some("https://img/x.jpg".into()), duration: Some(180.0), start_at: 12.0 };
        let v = load_request(&m, "sess-1", 7, true);
        assert_eq!(v["type"], "LOAD");
        assert_eq!(v["sessionId"], "sess-1");
        assert_eq!(v["media"]["contentType"], "audio/mp4");
        assert_eq!(v["media"]["streamType"], "BUFFERED");
        assert_eq!(v["media"]["metadata"]["albumName"], "B");
        assert_eq!(v["currentTime"], 12.0);
        assert_eq!(v["autoplay"], true);
    }
}
