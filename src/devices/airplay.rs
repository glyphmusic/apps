//! AirPlay (RAOP, the classic AirPlay audio protocol): Glyph decodes the
//! song itself and streams it to the speaker.
//!
//! - RTSP over TCP: OPTIONS, ANNOUNCE (SDP), SETUP, RECORD, SET_PARAMETER
//!   (volume, title / artist / album, progress), FLUSH, TEARDOWN.
//! - Audio over UDP as RTP: 352 frames of 16-bit stereo at 44.1 kHz per
//!   packet, as uncompressed ALAC frames, unencrypted (`et=0`).
//! - Sync packets (control port, once a second) tie RTP time to the wall
//!   clock; the speaker's timing requests (timing port) are answered.
//!
//! Receivers that require pairing or a password (newer HomePods and Apple TVs
//! with "Require password" / "same Home") refuse the session; Glyph shows the
//! refusal instead of playing.

use super::{Device, Media, Session};
use crate::mpd::Status;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const RATE: u32 = 44_100;
/// Frames per RTP packet (the `352` in the SDP).
const FRAMES: usize = 352;
/// Receiver buffer when it does not say (`Audio-Latency`): 2 s.
const DEFAULT_LATENCY: u32 = 88_200;
/// How far ahead of the clock packets are sent.
const LEAD: u32 = RATE / 4;
const USER_AGENT: &str = "iTunes/11.0.4 (Windows; N)";

fn other(msg: impl Into<String>) -> std::io::Error {
    std::io::Error::other(msg.into())
}

fn refused(msg: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg.into())
}

/// Random enough for session ids, SSRCs and the first RTP timestamp.
fn rand_u32() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    h.finish() as u32
}

/// NTP time (seconds since 1900, 32.32 fixed point).
fn ntp_now() -> (u32, u32) {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs() + 2_208_988_800;
    let frac = ((d.subsec_nanos() as u64) << 32) / 1_000_000_000;
    (secs as u32, frac as u32)
}

// ------------------------------------------------------------ packets

/// Writes bits most significant first.
struct Bits {
    out: Vec<u8>,
    used: u32,
}

impl Bits {
    fn put(&mut self, value: u32, bits: u32) {
        for i in (0..bits).rev() {
            if self.used % 8 == 0 {
                self.out.push(0);
            }
            if (value >> i) & 1 == 1 {
                *self.out.last_mut().unwrap() |= 0x80 >> (self.used % 8);
            }
            self.used += 1;
        }
    }
}

/// One ALAC frame that stores `pcm` (interleaved stereo, exactly
/// [`FRAMES`] frames) uncompressed.
pub fn alac_frame(pcm: &[i16]) -> Vec<u8> {
    let mut b = Bits { out: Vec::with_capacity(3 + pcm.len() * 2), used: 0 };
    b.put(1, 3); // channels - 1: stereo
    b.put(0, 4);
    b.put(0, 8);
    b.put(0, 4);
    b.put(0, 1); // no sample count: the SDP's 352
    b.put(0, 2);
    b.put(1, 1); // not compressed
    for s in pcm {
        b.put(*s as u16 as u32, 16);
    }
    b.out
}

pub fn rtp_audio(seq: u16, ts: u32, ssrc: u32, first: bool, payload: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(12 + payload.len());
    p.extend_from_slice(&[0x80, if first { 0xe0 } else { 0x60 }]);
    p.extend_from_slice(&seq.to_be_bytes());
    p.extend_from_slice(&ts.to_be_bytes());
    p.extend_from_slice(&ssrc.to_be_bytes());
    p.extend_from_slice(payload);
    p
}

/// "Play `now - latency` now": ties RTP time to the wall clock.
pub fn sync_packet(now_ts: u32, latency: u32, first: bool) -> Vec<u8> {
    let (s, f) = ntp_now();
    let mut p = Vec::with_capacity(20);
    p.extend_from_slice(&[if first { 0x90 } else { 0x80 }, 0xd4, 0x00, 0x07]);
    p.extend_from_slice(&now_ts.wrapping_sub(latency).to_be_bytes());
    p.extend_from_slice(&s.to_be_bytes());
    p.extend_from_slice(&f.to_be_bytes());
    p.extend_from_slice(&now_ts.to_be_bytes());
    p
}

/// Answer to a timing request (`0xd2`), `None` for anything else.
pub fn timing_reply(req: &[u8]) -> Option<Vec<u8>> {
    if req.len() < 32 || req[1] & 0x7f != 0x52 {
        return None;
    }
    let (s, f) = ntp_now();
    let mut p = vec![0x80, 0xd3, 0x00, 0x07, 0, 0, 0, 0];
    p.extend_from_slice(&req[24..32]); // their send time
    for _ in 0..2 {
        p.extend_from_slice(&s.to_be_bytes());
        p.extend_from_slice(&f.to_be_bytes());
    }
    Some(p)
}

/// DMAP item with title / artist / album for SET_PARAMETER.
pub fn dmap_meta(m: &Media) -> Vec<u8> {
    fn tag(out: &mut Vec<u8>, name: &[u8; 4], value: &[u8]) {
        out.extend_from_slice(name);
        out.extend_from_slice(&(value.len() as u32).to_be_bytes());
        out.extend_from_slice(value);
    }
    let mut items = Vec::new();
    tag(&mut items, b"minm", m.title.as_bytes());
    tag(&mut items, b"asar", m.artist.as_bytes());
    tag(&mut items, b"asal", m.album.as_deref().unwrap_or("").as_bytes());
    let mut out = Vec::new();
    tag(&mut out, b"mlit", &items);
    out
}

// ------------------------------------------------------------ RTSP

struct Rtsp {
    reader: BufReader<TcpStream>,
    stream: TcpStream,
    cseq: u32,
    session: Option<String>,
    client: String,
}

struct Reply {
    headers: Vec<(String, String)>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

impl Rtsp {
    fn request(&mut self, method: &str, uri: &str, headers: &[(&str, String)], body: Option<(&str, &[u8])>) -> std::io::Result<Reply> {
        self.cseq += 1;
        let mut req = format!("{method} {uri} RTSP/1.0\r\nCSeq: {}\r\nUser-Agent: {USER_AGENT}\r\nClient-Instance: {c}\r\nDACP-ID: {c}\r\n", self.cseq, c = self.client);
        if let Some(s) = &self.session {
            req.push_str(&format!("Session: {s}\r\n"));
        }
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        let mut bytes = Vec::new();
        if let Some((ct, b)) = body {
            req.push_str(&format!("Content-Type: {ct}\r\nContent-Length: {}\r\n", b.len()));
            bytes.extend_from_slice(b);
        }
        req.push_str("\r\n");
        let mut out = req.into_bytes();
        out.extend_from_slice(&bytes);
        self.stream.write_all(&out)?;

        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::ConnectionAborted, "AirPlay device closed the connection"));
        }
        let status: u16 = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or_else(|| other(format!("bad RTSP reply: {}", line.trim())))?;
        let mut headers = Vec::new();
        loop {
            line.clear();
            if self.reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
                break;
            }
            if let Some((k, v)) = line.split_once(':') {
                headers.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
        let reply = Reply { headers };
        if let Some(n) = reply.header("Content-Length").and_then(|v| v.parse::<usize>().ok()) {
            let mut skip = vec![0u8; n.min(1 << 20)];
            self.reader.read_exact(&mut skip)?;
        }
        match status {
            200 => Ok(reply),
            401 => Err(refused("this AirPlay device asks for a password, which Glyph does not support yet")),
            403 | 470 => Err(refused("this AirPlay device needs pairing (AirPlay 2), which Glyph does not support yet")),
            s => Err(other(format!("AirPlay {method}: RTSP {s}"))),
        }
    }
}

// ------------------------------------------------------------ the stream

#[derive(Clone, Copy, PartialEq)]
enum Run {
    Play,
    Pause,
    Ended,
    Failed,
}

/// Shared between the session (RTSP) and the sender thread.
struct Stream {
    run: Run,
    /// Seek here, then play (resume after a pause is a seek too: the FLUSH
    /// dropped what the speaker had buffered).
    seek_to: Option<f64>,
    seq: u16,
    /// RTP time of the next packet.
    next_ts: u32,
    /// Clock anchor: RTP time `anchor_ts` was "now" at `anchor_at`.
    anchor_ts: u32,
    anchor_at: Instant,
    /// Song position (s) of RTP time `base_ts`.
    base_secs: f64,
    base_ts: u32,
    /// Clock time at the pause (the clock stands still).
    paused_clock: Option<u32>,
}

impl Stream {
    fn clock(&self) -> u32 {
        self.paused_clock.unwrap_or_else(|| self.anchor_ts.wrapping_add((self.anchor_at.elapsed().as_secs_f64() * RATE as f64) as u32))
    }

    fn position(&self, latency: u32) -> f64 {
        let played = self.clock().wrapping_sub(self.base_ts).wrapping_sub(latency) as i32;
        self.base_secs + played.max(0) as f64 / RATE as f64
    }
}

struct Sender {
    shared: Arc<Mutex<Stream>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    timing: Option<std::thread::JoinHandle<()>>,
    latency: u32,
    duration: Option<f64>,
    uri: String,
}

impl Sender {
    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for t in [self.thread.take(), self.timing.take()].into_iter().flatten() {
            let _ = t.join();
        }
    }
}

fn send_loop(mut pcm: glyph_audio::pcm::Pcm, audio: UdpSocket, control: UdpSocket, remote_control: SocketAddr, shared: Arc<Mutex<Stream>>, stop: Arc<AtomicBool>, latency: u32) {
    let ssrc = rand_u32();
    let mut first = true;
    let mut eof = false;
    let mut last_sync = Instant::now() - Duration::from_secs(2);
    let mut frames: Vec<i16> = Vec::with_capacity(FRAMES * 2);
    while !stop.load(Ordering::SeqCst) {
        {
            let mut st = shared.lock().unwrap();
            if let Some(secs) = st.seek_to.take() {
                match pcm.seek(secs) {
                    Ok(landed) => st.base_secs = landed,
                    Err(e) => {
                        tracing::debug!("airplay seek: {e}");
                        st.base_secs = secs;
                    }
                }
                // The clock jumps to the next packet: speakers follow the
                // sync packet that comes with the marked first packet.
                st.anchor_ts = st.next_ts;
                st.anchor_at = Instant::now();
                st.base_ts = st.next_ts;
                st.paused_clock = None;
                st.run = Run::Play;
                eof = false;
                first = true;
                last_sync = Instant::now() - Duration::from_secs(2);
            }
            if st.run != Run::Play {
                drop(st);
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            let clock = st.clock();
            if last_sync.elapsed() >= Duration::from_secs(1) {
                let _ = control.send_to(&sync_packet(clock, latency, first), remote_control);
                last_sync = Instant::now();
            }
            while !eof && (st.next_ts.wrapping_sub(clock) as i32) < LEAD as i32 {
                frames.clear();
                let n = pcm.read(&mut frames, FRAMES);
                if n == 0 {
                    eof = true;
                    break;
                }
                frames.resize(FRAMES * 2, 0);
                let packet = rtp_audio(st.seq, st.next_ts, ssrc, first, &alac_frame(&frames));
                if let Err(e) = audio.send(&packet) {
                    tracing::debug!("airplay send: {e}");
                    st.run = Run::Failed;
                    break;
                }
                first = false;
                st.seq = st.seq.wrapping_add(1);
                st.next_ts = st.next_ts.wrapping_add(FRAMES as u32);
                if n < FRAMES {
                    eof = true;
                }
            }
            // Ended once the speaker played the last packet.
            if eof && (clock.wrapping_sub(st.next_ts).wrapping_sub(latency) as i32) >= 0 {
                st.run = Run::Ended;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn timing_loop(sock: UdpSocket, stop: Arc<AtomicBool>) {
    let _ = sock.set_read_timeout(Some(Duration::from_millis(300)));
    let mut buf = [0u8; 128];
    while !stop.load(Ordering::SeqCst) {
        if let Ok((n, from)) = sock.recv_from(&mut buf) {
            if let Some(reply) = timing_reply(&buf[..n]) {
                let _ = sock.send_to(&reply, from);
            }
        }
    }
}

// ------------------------------------------------------------ the session

pub struct Client {
    rtsp: Rtsp,
    remote: IpAddr,
    local: IpAddr,
    sender: Option<Sender>,
    volume: Option<u8>,
    last_keepalive: Instant,
}

impl Client {
    pub fn connect(d: &Device) -> std::io::Result<Self> {
        let stream = TcpStream::connect_timeout(&SocketAddr::new(d.addr, d.port), Duration::from_secs(3))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_nodelay(true)?;
        let local = stream.local_addr()?.ip();
        let client = format!("{:08X}{:08X}", rand_u32(), rand_u32());
        let mut rtsp = Rtsp { reader: BufReader::new(stream.try_clone()?), stream, cseq: 0, session: None, client };
        rtsp.request("OPTIONS", "*", &[], None)?;
        Ok(Self { rtsp, remote: d.addr, local, sender: None, volume: None, last_keepalive: Instant::now() })
    }

    fn end_stream(&mut self) {
        if let Some(mut s) = self.sender.take() {
            s.shutdown();
            let uri = s.uri.clone();
            let _ = self.rtsp.request("TEARDOWN", &uri, &[], None);
            self.rtsp.session = None;
        }
    }

    fn rtp_info(&self) -> String {
        self.sender.as_ref().map(|s| {
            let st = s.shared.lock().unwrap();
            format!("seq={};rtptime={}", st.seq, st.next_ts)
        }).unwrap_or_default()
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let Some(uri) = self.sender.as_ref().map(|s| s.uri.clone()) else { return Ok(()) };
        let info = self.rtp_info();
        self.rtsp.request("FLUSH", &uri, &[("RTP-Info", info)], None).map(|_| ())
    }

    /// "progress: start/current/end" in RTP time, for the speaker's display.
    fn send_progress(&mut self) {
        let Some(s) = self.sender.as_ref() else { return };
        let Some(dur) = s.duration else { return };
        let (uri, latency) = (s.uri.clone(), s.latency);
        let (start, cur, end) = {
            let st = s.shared.lock().unwrap();
            let start = st.base_ts.wrapping_sub((st.base_secs * RATE as f64) as u32);
            let cur = start.wrapping_add((st.position(latency) * RATE as f64) as u32);
            (start, cur, start.wrapping_add((dur * RATE as f64) as u32))
        };
        let body = format!("progress: {start}/{cur}/{end}\r\n");
        let _ = self.rtsp.request("SET_PARAMETER", &uri, &[], Some(("text/parameters", body.as_bytes())));
    }

    fn set_volume(&mut self, v: u8) -> std::io::Result<()> {
        let Some(uri) = self.sender.as_ref().map(|s| s.uri.clone()) else { return Ok(()) };
        // -30 dB (quiet) .. 0 dB (full); -144 is mute.
        let db = if v == 0 { -144.0 } else { -30.0 + 30.0 * v.min(100) as f64 / 100.0 };
        let body = format!("volume: {db:.6}\r\n");
        self.rtsp.request("SET_PARAMETER", &uri, &[], Some(("text/parameters", body.as_bytes()))).map(|_| ())
    }
}

impl Session for Client {
    fn load(&mut self, m: &Media, start: bool) -> std::io::Result<()> {
        self.end_stream();
        let mut pcm = glyph_audio::pcm::Pcm::open(&m.url, Some(&m.mime), RATE).map_err(other)?;
        let mut base_secs = 0.0;
        if m.start_at > 0.5 {
            base_secs = pcm.seek(m.start_at).unwrap_or(0.0);
        }

        let sid = rand_u32();
        let ip = |a: IpAddr| match a {
            IpAddr::V4(_) => ("IP4", a.to_string()),
            IpAddr::V6(_) => ("IP6", a.to_string()),
        };
        let (fam, local) = ip(self.local);
        let (rfam, remote) = ip(self.remote);
        let uri = format!("rtsp://{}/{sid}", if self.local.is_ipv6() { format!("[{local}]") } else { local.clone() });
        let sdp = format!(
            "v=0\r\no=iTunes {sid} 0 IN {fam} {local}\r\ns=iTunes\r\nc=IN {rfam} {remote}\r\nt=0 0\r\nm=audio 0 RTP/AVP 96\r\na=rtpmap:96 AppleLossless\r\na=fmtp:96 {FRAMES} 0 16 40 10 14 2 255 0 0 {RATE}\r\n"
        );
        self.rtsp.request("ANNOUNCE", &uri, &[], Some(("application/sdp", sdp.as_bytes())))?;

        let bind = SocketAddr::new(self.local, 0);
        let control = UdpSocket::bind(bind)?;
        let timing = UdpSocket::bind(bind)?;
        let transport = format!(
            "RTP/AVP/UDP;unicast;interleaved=0-1;mode=record;control_port={};timing_port={}",
            control.local_addr()?.port(),
            timing.local_addr()?.port()
        );
        let stop = Arc::new(AtomicBool::new(false));
        let timing_thread = {
            let (sock, stop) = (timing, stop.clone());
            std::thread::Builder::new().name("glyph-airplay-timing".into()).spawn(move || timing_loop(sock, stop))?
        };
        let setup = match self.rtsp.request("SETUP", &uri, &[("Transport", transport)], None) {
            Ok(r) => r,
            Err(e) => {
                stop.store(true, Ordering::SeqCst);
                let _ = timing_thread.join();
                return Err(e);
            }
        };
        self.rtsp.session = setup.header("Session").map(|s| s.split(';').next().unwrap_or(s).trim().to_string());
        let port_of = |key: &str| setup.header("Transport").and_then(|t| t.split(';').find_map(|p| p.trim().strip_prefix(key)?.strip_prefix('=')?.parse::<u16>().ok()));
        let server_port = port_of("server_port").ok_or_else(|| other("AirPlay SETUP without server_port"))?;
        let control_port = port_of("control_port").unwrap_or(server_port + 1);

        let seq = rand_u32() as u16;
        let ts = rand_u32();
        let record = self.rtsp.request("RECORD", &uri, &[("Range", "npt=0-".into()), ("RTP-Info", format!("seq={seq};rtptime={ts}"))], None)?;
        let latency = record.header("Audio-Latency").and_then(|v| v.parse::<u32>().ok()).filter(|l| *l > 0).unwrap_or(DEFAULT_LATENCY);

        let audio = UdpSocket::bind(bind)?;
        audio.connect(SocketAddr::new(self.remote, server_port))?;
        let now = Instant::now();
        let shared = Arc::new(Mutex::new(Stream {
            run: if start { Run::Play } else { Run::Pause },
            seek_to: None,
            seq,
            next_ts: ts,
            anchor_ts: ts,
            anchor_at: now,
            base_secs,
            base_ts: ts,
            paused_clock: if start { None } else { Some(ts) },
        }));
        let thread = {
            let (shared, stop) = (shared.clone(), stop.clone());
            let remote_control = SocketAddr::new(self.remote, control_port);
            std::thread::Builder::new().name("glyph-airplay".into()).spawn(move || send_loop(pcm, audio, control, remote_control, shared, stop, latency))?
        };
        self.sender = Some(Sender { shared, stop, thread: Some(thread), timing: Some(timing_thread), latency, duration: m.duration, uri: uri.clone() });

        if let Some(v) = self.volume {
            let _ = self.set_volume(v);
        }
        let meta = dmap_meta(m);
        let _ = self.rtsp.request("SET_PARAMETER", &uri, &[("RTP-Info", format!("rtptime={ts}"))], Some(("application/x-dmap-tagged", &meta)));
        self.send_progress();
        Ok(())
    }

    fn pause(&mut self, pause: bool) -> std::io::Result<()> {
        let Some(s) = self.sender.as_ref() else { return Ok(()) };
        let latency = s.latency;
        if pause {
            {
                let mut st = s.shared.lock().unwrap();
                if st.run != Run::Play {
                    return Ok(());
                }
                st.paused_clock = Some(st.clock());
                st.run = Run::Pause;
            }
            self.flush()
        } else {
            let mut st = s.shared.lock().unwrap();
            if st.run == Run::Pause {
                let at = st.position(latency);
                st.seek_to = Some(at);
            }
            Ok(())
        }
    }

    fn seek(&mut self, secs: f64) -> std::io::Result<()> {
        let Some(s) = self.sender.as_ref() else { return Ok(()) };
        let playing = {
            let mut st = s.shared.lock().unwrap();
            let playing = st.run == Run::Play;
            // Hold the sender while the speaker drops its buffer.
            st.paused_clock = Some(st.clock());
            st.run = Run::Pause;
            playing
        };
        self.flush()?;
        let Some(s) = self.sender.as_ref() else { return Ok(()) };
        let mut st = s.shared.lock().unwrap();
        if playing {
            st.seek_to = Some(secs.max(0.0));
        } else {
            // Stays paused at the new place; play seeks there.
            st.base_secs = secs.max(0.0);
            st.base_ts = st.clock().wrapping_sub(s.latency);
        }
        drop(st);
        self.send_progress();
        Ok(())
    }

    fn volume(&mut self, v: u8) -> std::io::Result<()> {
        self.volume = Some(v);
        self.set_volume(v)
    }

    fn stop(&mut self) -> std::io::Result<()> {
        self.end_stream();
        Ok(())
    }

    fn status(&mut self) -> std::io::Result<Status> {
        let Some(s) = self.sender.as_ref() else {
            return Ok(Status { state: "stop".into(), elapsed: None, duration: None, volume: self.volume });
        };
        let (run, elapsed) = {
            let st = s.shared.lock().unwrap();
            (st.run, st.position(s.latency))
        };
        let duration = s.duration;
        // Paused sessions see no audio; keep the RTSP connection alive.
        if run != Run::Play && self.last_keepalive.elapsed() > Duration::from_secs(25) {
            self.last_keepalive = Instant::now();
            self.rtsp.request("OPTIONS", "*", &[], None)?;
        }
        let state = match run {
            Run::Play => "play",
            Run::Pause => "pause",
            Run::Ended => "stop",
            Run::Failed => "error",
        };
        Ok(Status { state: state.into(), elapsed: Some(duration.map_or(elapsed, |d| elapsed.min(d))), duration, volume: self.volume })
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.end_stream();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncompressed_alac_frame() {
        let pcm = [0x1234i16, -2].repeat(FRAMES);
        let f = alac_frame(&pcm);
        // 23 header bits + 352 * 32 sample bits.
        assert_eq!(f.len(), (23 + FRAMES * 32).div_ceil(8));
        // Header: 001 0000 00000000 0000 0 00 1, then 0x1234 big-endian.
        assert_eq!(f[0], 0b0010_0000);
        assert_eq!(f[1], 0);
        assert_eq!(f[2], 0b0000_0010 | (0x1234u16 >> 15) as u8);
        assert_eq!(f[3], ((0x1234u16 >> 7) & 0xff) as u8);
    }

    #[test]
    fn rtp_and_sync_packets() {
        let p = rtp_audio(7, 1000, 42, true, &[1, 2, 3]);
        assert_eq!(&p[..4], &[0x80, 0xe0, 0, 7]);
        assert_eq!(u32::from_be_bytes(p[4..8].try_into().unwrap()), 1000);
        assert_eq!(p.len(), 15);
        let s = sync_packet(100_000, DEFAULT_LATENCY, true);
        assert_eq!(&s[..4], &[0x90, 0xd4, 0, 7]);
        assert_eq!(u32::from_be_bytes(s[4..8].try_into().unwrap()), 100_000 - DEFAULT_LATENCY);
        assert_eq!(u32::from_be_bytes(s[16..20].try_into().unwrap()), 100_000);
    }

    #[test]
    fn answers_timing_requests() {
        let mut req = vec![0x80, 0xd2, 0, 7];
        req.resize(32, 0);
        req[24..32].copy_from_slice(&[9; 8]);
        let r = timing_reply(&req).unwrap();
        assert_eq!((&r[..2], &r[8..16]), (&[0x80u8, 0xd3][..], &[9u8; 8][..]));
        assert!(timing_reply(&[0x80, 0xd4, 0, 7]).is_none());
    }

    #[test]
    fn position_follows_the_clock() {
        let now = Instant::now();
        let st = Stream { run: Run::Pause, seek_to: None, seq: 0, next_ts: 0, anchor_ts: 0, anchor_at: now, base_secs: 10.0, base_ts: 0, paused_clock: Some(DEFAULT_LATENCY + RATE * 3) };
        assert!((st.position(DEFAULT_LATENCY) - 13.0).abs() < 1e-9);
        let early = Stream { paused_clock: Some(RATE), ..st };
        assert_eq!(early.position(DEFAULT_LATENCY), 10.0);
    }

    #[test]
    fn metadata_item() {
        let m = Media { url: String::new(), mime: String::new(), title: "Song".into(), artist: "Me".into(), album: None, art: None, duration: None, start_at: 0.0 };
        let d = dmap_meta(&m);
        assert_eq!(&d[..4], b"mlit");
        assert_eq!(u32::from_be_bytes(d[4..8].try_into().unwrap()) as usize, d.len() - 8);
    }
}
