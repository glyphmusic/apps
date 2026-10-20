//! Playback devices on the local network: Glyph looks for them on its own
//! (mDNS / DNS-SD and SSDP) and plays on the one the user picks in the
//! output menu, staying the remote (play / pause, seek, volume, next).
//!
//! - MPD (`_mpd._tcp`): handed to the MPD output ([`crate::mpd`]).
//! - UPnP / DLNA MediaRenderers (TVs, receivers, speakers) and Sonos: SSDP,
//!   AVTransport + RenderingControl ([`upnp`]).
//! - Google Cast (`_googlecast._tcp`): Default Media Receiver ([`cast`]).
//! - AirPlay (`_airplay._tcp` / `_raop._tcp`): listed, not playable yet.
//!
//! Everything is plain Rust on std sockets (plus rustls for Cast), so it
//! works the same on Linux, macOS and Windows without system services.

pub mod cast;
pub mod http;
pub mod mdns;
pub mod media;
pub mod ssdp;
pub mod upnp;

use crate::mpd::{Cmd, Status};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What kind of device (and protocol) it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Chromecast,
    Sonos,
    Upnp,
    Mpd,
    AirPlay,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Chromecast, Kind::Sonos, Kind::Upnp, Kind::Mpd, Kind::AirPlay];

    /// Glyph can play on it.
    pub fn playable(self) -> bool {
        self != Kind::AirPlay
    }

}

/// One found device.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    /// Stable across scans: `cast:<id>`, `upnp:<udn>`, `mpd:<ip>:<port>`, ...
    pub id: String,
    pub kind: Kind,
    pub name: String,
    pub model: String,
    pub addr: IpAddr,
    pub port: u16,
    /// Icon name (assets/icons/ui).
    pub icon: &'static str,
    /// UPnP / Sonos: what the description says.
    pub upnp: Option<upnp::Description>,
}

impl Device {
    /// `host:port` for the MPD output.
    pub fn mpd_address(&self) -> String {
        match self.addr {
            IpAddr::V6(a) => format!("[{a}]:{}", self.port),
            IpAddr::V4(a) => format!("{a}:{}", self.port),
        }
    }
}

/// A song as a device gets it.
#[derive(Debug, Clone, PartialEq)]
pub struct Media {
    pub url: String,
    pub mime: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub art: Option<String>,
    /// Seconds; `None` for live streams.
    pub duration: Option<f64>,
    /// Start this far in (moving playback from this computer).
    pub start_at: f64,
}

/// A connection to a device that plays (UPnP renderer, Cast receiver).
pub trait Session: Send {
    fn load(&mut self, m: &Media, start: bool) -> std::io::Result<()>;
    fn pause(&mut self, pause: bool) -> std::io::Result<()>;
    fn seek(&mut self, secs: f64) -> std::io::Result<()>;
    fn volume(&mut self, v: u8) -> std::io::Result<()>;
    fn stop(&mut self) -> std::io::Result<()>;
    /// `state`: "play" | "pause" | "stop" (ended) | "load" | "error" (the
    /// device could not play the song) | "closed" (another app took over).
    fn status(&mut self) -> std::io::Result<Status>;
}

fn connect(d: &Device) -> std::io::Result<Box<dyn Session>> {
    match d.kind {
        Kind::Chromecast => Ok(Box::new(cast::Client::connect(d)?)),
        Kind::Upnp | Kind::Sonos => Ok(Box::new(upnp::Renderer::connect(d)?)),
        Kind::Mpd | Kind::AirPlay => Err(std::io::Error::other("not playable here")),
    }
}

// ------------------------------------------------------------ sockets

/// One UDP socket per IPv4 network adapter, so searches leave through every
/// one of them: with VPN (Tailscale), VM (Hyper-V, VirtualBox) or WSL
/// adapters the default route often is not the home network.
pub fn lan_sockets(ttl: u32) -> Vec<std::net::UdpSocket> {
    use std::net::{Ipv4Addr, SocketAddr};
    let mut ips: Vec<Ipv4Addr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .filter_map(|i| match i.ip() {
            IpAddr::V4(a) if !a.is_link_local() => Some(a),
            _ => None,
        })
        .collect();
    ips.sort();
    ips.dedup();
    let open = |ip: Ipv4Addr| -> std::io::Result<std::net::UdpSocket> {
        let s = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, Some(socket2::Protocol::UDP))?;
        s.bind(&SocketAddr::from((ip, 0)).into())?;
        if !ip.is_unspecified() {
            s.set_multicast_if_v4(&ip)?;
        }
        s.set_multicast_ttl_v4(ttl)?;
        s.set_nonblocking(true)?;
        Ok(s.into())
    };
    let mut out: Vec<std::net::UdpSocket> = ips.into_iter().filter_map(|ip| open(ip).ok()).collect();
    if out.is_empty() {
        out.extend(open(Ipv4Addr::UNSPECIFIED).ok());
    }
    out
}

/// Reads datagrams from all `socks` until `until`.
pub fn recv_all(socks: &[std::net::UdpSocket], until: Instant, mut f: impl FnMut(&[u8], std::net::SocketAddr)) {
    let mut buf = [0u8; 9000];
    while Instant::now() < until {
        let mut got = false;
        for s in socks {
            while let Ok((n, from)) = s.recv_from(&mut buf) {
                got = true;
                f(&buf[..n], from);
            }
        }
        if !got {
            std::thread::sleep(Duration::from_millis(15));
        }
    }
}

// ------------------------------------------------------------ discovery

const MDNS_TYPES: [&str; 4] = ["_googlecast._tcp.local", "_mpd._tcp.local", "_airplay._tcp.local", "_raop._tcp.local"];
/// Not seen for this long: gone.
const FORGET_AFTER: Duration = Duration::from_secs(200);
const SCAN_EVERY: Duration = Duration::from_secs(60);

struct Shared {
    devices: Mutex<Vec<(Device, Instant)>>,
    /// Parsed descriptions per location (a TV's description rarely changes).
    descriptions: Mutex<HashMap<String, Option<upnp::Description>>>,
    searching: AtomicBool,
    generation: AtomicU64,
    stop: AtomicBool,
}

/// Background search, repeated every minute and on demand.
pub struct Discovery {
    shared: Arc<Shared>,
    wake: flume::Sender<()>,
    last_rescan: Mutex<Instant>,
}

impl Discovery {
    pub fn start() -> Self {
        let shared = Arc::new(Shared {
            devices: Mutex::new(Vec::new()),
            descriptions: Mutex::new(HashMap::new()),
            searching: AtomicBool::new(true),
            generation: AtomicU64::new(1),
            stop: AtomicBool::new(false),
        });
        let (wake, rx) = flume::unbounded::<()>();
        let sh = shared.clone();
        let _ = std::thread::Builder::new().name("glyph-discovery".into()).spawn(move || {
            while !sh.stop.load(Ordering::Relaxed) {
                sh.searching.store(true, Ordering::Relaxed);
                sh.generation.fetch_add(1, Ordering::Relaxed);
                scan(&sh);
                sh.searching.store(false, Ordering::Relaxed);
                sh.generation.fetch_add(1, Ordering::Relaxed);
                match rx.recv_timeout(SCAN_EVERY) {
                    Ok(()) => while rx.try_recv().is_ok() {},
                    Err(flume::RecvTimeoutError::Timeout) => {}
                    Err(flume::RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        Self { shared, wake, last_rescan: Mutex::new(Instant::now()) }
    }

    /// Searches again now (the output menu opened); at most every 10 s.
    pub fn rescan(&self) {
        let mut last = self.last_rescan.lock().unwrap();
        if last.elapsed() > Duration::from_secs(10) && !self.searching() {
            *last = Instant::now();
            let _ = self.wake.send(());
        }
    }

    /// Searches again now (refresh button), unless a search is running.
    pub fn rescan_now(&self) {
        if !self.searching() {
            *self.last_rescan.lock().unwrap() = Instant::now();
            let _ = self.wake.send(());
        }
    }

    /// Found devices, by kind then name.
    pub fn devices(&self) -> Vec<Device> {
        let mut v: Vec<Device> = self.shared.devices.lock().unwrap().iter().map(|(d, _)| d.clone()).collect();
        v.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        v
    }

    pub fn find(&self, id: &str) -> Option<Device> {
        self.shared.devices.lock().unwrap().iter().find(|(d, _)| d.id == id).map(|(d, _)| d.clone())
    }

    pub fn searching(&self) -> bool {
        self.shared.searching.load(Ordering::Relaxed)
    }

    /// Changes whenever the list or the searching state changes.
    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Relaxed)
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        let _ = self.wake.send(());
    }
}

/// Lowercase alphanumerics only, for matching names across protocols.
fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Devices from mDNS services.
pub fn from_mdns(services: &[mdns::Service]) -> Vec<Device> {
    let mut out: Vec<Device> = Vec::new();
    for s in services {
        let Some(addr) = s.addr() else { continue };
        let d = match s.service_type.as_str() {
            "_googlecast._tcp.local" => {
                let name = s.txt("fn").map(str::trim).filter(|n| !n.is_empty()).unwrap_or(s.label.trim()).to_string();
                let model = s.txt("md").unwrap_or_default().to_string();
                let m = model.to_lowercase();
                let icon = if m.contains("home") || m.contains("nest") || m.contains("speaker") || m.contains("audio") || m.contains("group") { "speaker" } else { "cast" };
                let id = s.txt("id").map(|i| format!("cast:{i}")).unwrap_or_else(|| format!("cast:{addr}:{}", s.port));
                Device { id, kind: Kind::Chromecast, name, model, addr, port: s.port, icon, upnp: None }
            }
            "_mpd._tcp.local" => Device {
                id: format!("mpd:{addr}:{}", s.port),
                kind: Kind::Mpd,
                name: s.label.clone(),
                model: "MPD".into(),
                addr,
                port: s.port,
                icon: "radio",
                upnp: None,
            },
            "_airplay._tcp.local" | "_raop._tcp.local" => {
                // RAOP instances are "MAC@Name".
                let name = s.label.split_once('@').map(|(_, n)| n).unwrap_or(&s.label).to_string();
                Device {
                    id: format!("airplay:{addr}"),
                    kind: Kind::AirPlay,
                    name,
                    model: s.txt("model").or_else(|| s.txt("am")).unwrap_or_default().to_string(),
                    addr,
                    port: s.port,
                    icon: "airplay",
                    upnp: None,
                }
            }
            _ => continue,
        };
        if !out.iter().any(|o| o.id == d.id) {
            out.push(d);
        }
    }
    out
}

/// A device from a UPnP description (`None` when it cannot play).
pub fn from_upnp(desc: upnp::Description, location: &str) -> Option<Device> {
    desc.av_transport.as_ref()?;
    let url = http::Url::parse(location)?;
    let addr = url.ip()?;
    let sonos = desc.is_sonos();
    let looks_tv = {
        let s = format!("{} {}", desc.friendly_name, desc.model).to_lowercase();
        s.contains("tv") || s.contains("bravia") || s.contains("television")
    };
    Some(Device {
        id: format!("upnp:{}", if desc.udn.is_empty() { location } else { desc.udn.as_str() }),
        kind: if sonos { Kind::Sonos } else { Kind::Upnp },
        name: desc.friendly_name.trim().to_string(),
        model: if desc.model.is_empty() { desc.manufacturer.clone() } else { desc.model.clone() },
        addr,
        port: url.port,
        icon: if sonos { "speaker" } else if looks_tv { "tv" } else { "monitor-speaker" },
        upnp: Some(desc),
    })
}

/// AirPlay entries for devices Glyph can already play through another
/// protocol (Sonos, TVs, Cast) only add noise: drop them.
pub fn merge(mut found: Vec<Device>) -> Vec<Device> {
    let playable: Vec<(IpAddr, String)> = found.iter().filter(|d| d.kind.playable()).map(|d| (d.addr, norm(&d.name))).collect();
    found.retain(|d| d.kind != Kind::AirPlay || !playable.iter().any(|(a, n)| *a == d.addr || *n == norm(&d.name)));
    let mut out: Vec<Device> = Vec::new();
    for d in found {
        // _airplay and _raop of the same device: one entry.
        if !out.iter().any(|o| o.id == d.id) {
            out.push(d);
        }
    }
    out
}

fn scan(sh: &Shared) {
    let (mdns_found, ssdp_found) = std::thread::scope(|s| {
        let m = s.spawn(|| mdns::browse(&MDNS_TYPES, Duration::from_millis(2500)));
        let u = s.spawn(|| ssdp::search(&[ssdp::MEDIA_RENDERER, ssdp::ZONE_PLAYER], Duration::from_millis(2500)));
        (m.join().unwrap_or_default(), u.join().unwrap_or_default())
    });
    let mut found = from_mdns(&mdns_found);

    // Descriptions not seen before: fetch in parallel.
    let new: Vec<String> = {
        let known = sh.descriptions.lock().unwrap();
        ssdp_found.iter().map(|r| r.location.clone()).filter(|l| !known.contains_key(l)).collect()
    };
    let fetched: Vec<(String, Option<upnp::Description>)> = std::thread::scope(|s| {
        let handles: Vec<_> = new
            .iter()
            .map(|loc| {
                s.spawn(move || {
                    let d = http::get(loc, Duration::from_secs(4)).ok().filter(|r| r.status == 200).and_then(|r| upnp::parse_description(&r.text(), loc));
                    (loc.clone(), d)
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    {
        let mut known = sh.descriptions.lock().unwrap();
        for (loc, d) in fetched {
            // A failed fetch is tried again on the next scan.
            if d.is_some() {
                known.insert(loc, d);
            }
        }
        for r in &ssdp_found {
            if let Some(Some(desc)) = known.get(&r.location) {
                if let Some(d) = from_upnp(desc.clone(), &r.location) {
                    if !found.iter().any(|f| f.id == d.id) {
                        found.push(d);
                    }
                }
            }
        }
    }
    let found = merge(found);

    let now = Instant::now();
    let mut list = sh.devices.lock().unwrap();
    let before: Vec<Device> = list.iter().map(|(d, _)| d.clone()).collect();
    for d in found {
        match list.iter_mut().find(|(x, _)| x.id == d.id) {
            Some(slot) => *slot = (d, now),
            None => list.push((d, now)),
        }
    }
    list.retain(|(_, seen)| now.duration_since(*seen) < FORGET_AFTER);
    let after: Vec<Device> = list.iter().map(|(d, _)| d.clone()).collect();
    if before != after {
        sh.generation.fetch_add(1, Ordering::Relaxed);
        tracing::debug!(count = after.len(), "network devices changed");
    }
}

// ------------------------------------------------------------ the output

enum Job {
    Cmd(Cmd),
    Load(Media, bool),
}

/// Playing on a network device: a worker thread holds the connection and
/// polls the device's state twice a second (same shape as the MPD output).
pub struct Output {
    pub device: Device,
    tx: flume::Sender<Job>,
    status: Arc<Mutex<Option<Status>>>,
    error: Arc<Mutex<Option<String>>>,
    files: Option<media::MediaServer>,
}

impl Output {
    pub fn start(device: Device) -> Self {
        let (tx, rx) = flume::unbounded::<Job>();
        let status: Arc<Mutex<Option<Status>>> = Default::default();
        let error: Arc<Mutex<Option<String>>> = Default::default();
        let (st, err, dev) = (status.clone(), error.clone(), device.clone());
        let _ = std::thread::Builder::new().name("glyph-device".into()).spawn(move || {
            let mut session: Option<Box<dyn Session>> = None;
            let mut pending: Option<(Media, bool)> = None;
            loop {
                let job = match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(j) => Some(j),
                    Err(flume::RecvTimeoutError::Timeout) => None,
                    Err(flume::RecvTimeoutError::Disconnected) => break,
                };
                // Only the newest song matters; volume / seek go through.
                let job = match job {
                    Some(Job::Load(m, start)) => {
                        pending = Some((m, start));
                        None
                    }
                    Some(Job::Cmd(Cmd::Play { url, start })) => {
                        let mime = media::guess_mime(&url, None);
                        pending = Some((Media { url, mime, title: String::new(), artist: String::new(), album: None, art: None, duration: None, start_at: 0.0 }, start));
                        None
                    }
                    Some(Job::Cmd(Cmd::Stop)) => {
                        if let Some(s) = session.as_mut() {
                            let _ = s.stop();
                        }
                        break;
                    }
                    other => other,
                };
                if session.is_none() {
                    match connect(&dev) {
                        Ok(s) => {
                            session = Some(s);
                            *err.lock().unwrap() = None;
                        }
                        Err(e) => {
                            tracing::debug!(device = %dev.name, "device connection: {e}");
                            *err.lock().unwrap() = Some(e.to_string());
                            *st.lock().unwrap() = None;
                            std::thread::sleep(Duration::from_secs(2));
                            continue;
                        }
                    }
                }
                let s = session.as_mut().unwrap();
                let r = (|| -> std::io::Result<()> {
                    if let Some((m, start)) = pending.take() {
                        *st.lock().unwrap() = Some(Status { state: "load".into(), elapsed: Some(m.start_at), duration: m.duration, volume: None });
                        if let Err(e) = s.load(&m, start) {
                            *st.lock().unwrap() = Some(Status { state: "error".into(), elapsed: None, duration: m.duration, volume: None });
                            return Err(e);
                        }
                    }
                    match job {
                        Some(Job::Cmd(Cmd::Pause(p))) => s.pause(p)?,
                        Some(Job::Cmd(Cmd::Seek(secs))) => s.seek(secs)?,
                        Some(Job::Cmd(Cmd::Volume(v))) => s.volume(v)?,
                        _ => {}
                    }
                    let now = s.status()?;
                    *st.lock().unwrap() = Some(now);
                    Ok(())
                })();
                match r {
                    Ok(()) => *err.lock().unwrap() = None,
                    Err(e) => {
                        tracing::debug!(device = %dev.name, "device: {e}");
                        *err.lock().unwrap() = Some(e.to_string());
                        // UPnP errors are per action; Cast I/O errors mean the
                        // connection is gone (refusals are InvalidData).
                        if dev.kind == Kind::Chromecast && e.kind() != std::io::ErrorKind::InvalidData {
                            session = None;
                        }
                    }
                }
            }
        });
        Self { device, tx, status, error, files: media::MediaServer::start().ok() }
    }

    pub fn send(&self, c: Cmd) {
        let _ = self.tx.send(Job::Cmd(c));
    }

    /// Hands a song over; `false` when the device cannot play this source
    /// (embedded players, files the server cannot reach).
    pub fn load(&self, track: &glyph_protocol::Track, src: &glyph_protocol::PlaybackSource, start: bool, start_at: f64) -> bool {
        let (url, mime) = match src {
            glyph_protocol::PlaybackSource::Stream { url, mime, .. } => (url, mime.as_deref()),
            glyph_protocol::PlaybackSource::Preview { url, .. } => (url, None),
            glyph_protocol::PlaybackSource::Embed { .. } => return false,
        };
        let mime = media::guess_mime(url, mime);
        // Cast devices open HTTPS themselves; the rest get plain HTTP from here.
        let direct = self.device.kind == Kind::Chromecast && url.starts_with("https://");
        let url = if direct { Some(url.clone()) } else { self.files.as_ref().and_then(|f| f.url_for(url, &mime, self.device.addr)) };
        let Some(url) = url else { return false };
        let preview = matches!(src, glyph_protocol::PlaybackSource::Preview { .. });
        let m = Media {
            url,
            mime,
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            art: track.cover_url.clone().filter(|u| u.starts_with("http")),
            duration: if preview { None } else { track.duration_ms.map(|ms| ms as f64 / 1000.0) },
            start_at,
        };
        // The old song's state is stale now ("ended", "error").
        *self.status.lock().unwrap() = None;
        let _ = self.tx.send(Job::Load(m, start));
        true
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

    fn svc(t: &str, label: &str, txt: &[(&str, &str)], ip: [u8; 4], port: u16) -> mdns::Service {
        mdns::Service {
            label: label.into(),
            service_type: t.into(),
            host: "h.local".into(),
            port,
            txt: txt.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            addrs: vec![IpAddr::from(ip)],
        }
    }

    /// Real network: `cargo test -p glyph live_scan -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_scan() {
        let sh = Shared {
            devices: Mutex::new(Vec::new()),
            descriptions: Mutex::new(HashMap::new()),
            searching: AtomicBool::new(true),
            generation: AtomicU64::new(0),
            stop: AtomicBool::new(false),
        };
        println!("adapters: {}", lan_sockets(1).len());
        scan(&sh);
        for (d, _) in sh.devices.lock().unwrap().iter() {
            println!("{:?} {:?} {} ({}) {}:{} {:?}", d.kind, d.icon, d.name, d.model, d.addr, d.port, d.upnp.as_ref().and_then(|u| u.av_transport.clone()));
        }
    }

    #[test]
    fn devices_from_mdns() {
        let found = from_mdns(&[
            svc("_googlecast._tcp.local", "Chromecast-abc", &[("id", "abc"), ("fn", "Wohnzimmer"), ("md", "Chromecast")], [10, 0, 0, 2], 8009),
            svc("_googlecast._tcp.local", "Google-Nest-Mini-x", &[("id", "nest"), ("fn", "Küche"), ("md", "Google Nest Mini")], [10, 0, 0, 3], 8009),
            svc("_mpd._tcp.local", "Music Player Daemon on pi", &[], [10, 0, 0, 4], 6600),
            svc("_raop._tcp.local", "A1B2C3D4E5F6@HomePod", &[("am", "AudioAccessory5,1")], [10, 0, 0, 5], 7000),
        ]);
        assert_eq!(found.len(), 4);
        assert_eq!((found[0].id.as_str(), found[0].name.as_str(), found[0].icon), ("cast:abc", "Wohnzimmer", "cast"));
        assert_eq!(found[1].icon, "speaker");
        assert_eq!((found[2].kind, found[2].mpd_address().as_str()), (Kind::Mpd, "10.0.0.4:6600"));
        assert_eq!((found[3].kind, found[3].name.as_str()), (Kind::AirPlay, "HomePod"));
    }

    #[test]
    fn airplay_duplicates_dropped() {
        let sonos = from_upnp(
            upnp::Description { friendly_name: "Küche".into(), manufacturer: "Sonos, Inc.".into(), av_transport: Some("http://10.0.0.9:1400/MediaRenderer/AVTransport/Control".into()), udn: "uuid:RINCON_1".into(), ..Default::default() },
            "http://10.0.0.9:1400/xml/device_description.xml",
        )
        .unwrap();
        assert_eq!((sonos.kind, sonos.icon, sonos.id.as_str()), (Kind::Sonos, "speaker", "upnp:uuid:RINCON_1"));
        let mut list = vec![sonos];
        list.extend(from_mdns(&[
            svc("_airplay._tcp.local", "Küche", &[], [10, 0, 0, 9], 7000),
            svc("_raop._tcp.local", "AABB@Küche", &[], [10, 0, 0, 9], 7000),
            svc("_airplay._tcp.local", "Apple TV", &[], [10, 0, 0, 10], 7000),
            svc("_raop._tcp.local", "CCDD@Apple TV", &[], [10, 0, 0, 10], 7000),
        ]));
        let merged = merge(list);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[1].name, "Apple TV");
        // Renderers without AVTransport cannot play.
        assert!(from_upnp(upnp::Description::default(), "http://10.0.0.1/").is_none());
    }
}
