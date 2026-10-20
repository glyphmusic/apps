//! UPnP AV MediaRenderer control (smart TVs, AV receivers, DLNA speakers,
//! Sonos): the device description, SOAP actions on AVTransport (play,
//! pause, seek) and RenderingControl (volume), DIDL-Lite metadata.
//!
//! Sonos quirks: speakers in a group only take transport commands on the
//! group coordinator (found through ZoneGroupTopology), and plain MP3 live
//! streams play as `x-rincon-mp3radio://`.

use super::http;
use super::{Media, Session};
use crate::mpd::Status;
use std::time::{Duration, Instant};

pub const AV_TRANSPORT: &str = "urn:schemas-upnp-org:service:AVTransport:1";
pub const RENDERING_CONTROL: &str = "urn:schemas-upnp-org:service:RenderingControl:1";
pub const ZONE_GROUP_TOPOLOGY: &str = "urn:schemas-upnp-org:service:ZoneGroupTopology:1";

const TIMEOUT: Duration = Duration::from_secs(5);

/// What Glyph needs from a device description.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Description {
    pub friendly_name: String,
    pub manufacturer: String,
    pub model: String,
    pub device_type: String,
    /// `uuid:…` of the device that renders.
    pub udn: String,
    /// Absolute control URLs.
    pub av_transport: Option<String>,
    pub rendering_control: Option<String>,
    pub zone_group_topology: Option<String>,
}

impl Description {
    pub fn is_sonos(&self) -> bool {
        self.manufacturer.to_ascii_lowercase().contains("sonos") || self.device_type.contains("ZonePlayer") || self.zone_group_topology.is_some()
    }
}

fn child_text<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> Option<&'a str> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == name).and_then(|c| c.text()).map(str::trim)
}

/// Parses a device description (`location` resolves relative URLs).
pub fn parse_description(xml: &str, location: &str) -> Option<Description> {
    let doc = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).ok()?;
    let root = doc.root_element();
    let base = root
        .children()
        .find(|c| c.is_element() && c.tag_name().name() == "URLBase")
        .and_then(|c| c.text())
        .map(|s| s.trim().to_string())
        .filter(|s| s.starts_with("http://"))
        .unwrap_or_else(|| location.to_string());
    let devices: Vec<roxmltree::Node> = root.descendants().filter(|n| n.is_element() && n.tag_name().name() == "device").collect();
    let top = *devices.first()?;
    let mut d = Description::default();
    // Sonos names the room in <roomName>; others in <friendlyName>.
    d.friendly_name = child_text(top, "roomName").or_else(|| child_text(top, "friendlyName")).unwrap_or_default().to_string();
    d.manufacturer = child_text(top, "manufacturer").unwrap_or_default().to_string();
    d.model = child_text(top, "modelName").unwrap_or_default().to_string();
    d.device_type = child_text(top, "deviceType").unwrap_or_default().to_string();
    d.udn = child_text(top, "UDN").unwrap_or_default().to_string();
    for dev in &devices {
        let services = dev
            .children()
            .filter(|c| c.is_element() && c.tag_name().name() == "serviceList")
            .flat_map(|l| l.children().filter(|c| c.is_element() && c.tag_name().name() == "service"));
        for s in services {
            let (Some(st), Some(ctl)) = (child_text(s, "serviceType"), child_text(s, "controlURL")) else { continue };
            let url = http::join(&base, ctl);
            let slot = match st.rsplit_once(':').map(|(t, _)| t) {
                Some("urn:schemas-upnp-org:service:AVTransport") => &mut d.av_transport,
                Some("urn:schemas-upnp-org:service:RenderingControl") => &mut d.rendering_control,
                Some("urn:schemas-upnp-org:service:ZoneGroupTopology") => &mut d.zone_group_topology,
                _ => continue,
            };
            if slot.is_none() {
                *slot = Some(url);
                if st.contains("AVTransport") {
                    // The UDN of the renderer itself (an embedded device on Sonos).
                    if let Some(u) = child_text(*dev, "UDN") {
                        if d.udn.is_empty() || !d.udn.contains("RINCON") {
                            d.udn = u.to_string();
                        }
                    }
                }
            }
        }
    }
    if d.friendly_name.is_empty() {
        d.friendly_name = d.model.clone();
    }
    Some(d)
}

// ------------------------------------------------------------ SOAP

pub fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c => o.push(c),
        }
    }
    o
}

pub fn soap_body(service: &str, action: &str, args: &[(&str, &str)]) -> String {
    let mut a = String::new();
    for (k, v) in args {
        a.push_str(&format!("<{k}>{}</{k}>", xml_escape(v)));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{service}\">{a}</u:{action}></s:Body></s:Envelope>"
    )
}

/// Text of the first element called `name` (any namespace).
pub fn xml_value(xml: &str, name: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let n = doc.descendants().find(|n| n.is_element() && n.tag_name().name() == name)?;
    Some(n.text().unwrap_or_default().to_string())
}

/// Runs one action; the response envelope.
pub fn soap(control: &str, service: &str, action: &str, args: &[(&str, &str)]) -> std::io::Result<String> {
    let body = soap_body(service, action, args);
    let sa = format!("\"{service}#{action}\"");
    let r = http::request("POST", control, &[("Content-Type", "text/xml; charset=\"utf-8\""), ("SOAPACTION", &sa)], body.as_bytes(), TIMEOUT)?;
    let text = r.text();
    if r.status != 200 {
        let code = xml_value(&text, "errorCode").unwrap_or_else(|| r.status.to_string());
        let desc = xml_value(&text, "errorDescription").unwrap_or_default();
        return Err(std::io::Error::other(format!("{action}: UPnP error {code} {desc}").trim().to_string()));
    }
    Ok(text)
}

/// `H:MM:SS(.fff)` -> seconds (`NOT_IMPLEMENTED` and junk -> `None`).
pub fn parse_time(s: &str) -> Option<f64> {
    let mut total = 0.0;
    let parts: Vec<&str> = s.trim().split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    for p in parts {
        total = total * 60.0 + p.parse::<f64>().ok()?;
    }
    Some(total)
}

pub fn format_time(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// DIDL-Lite item describing the song (many renderers refuse a URI without).
pub fn didl(m: &Media, uri: &str) -> String {
    let e = xml_escape;
    let mut item = format!("<dc:title>{}</dc:title><upnp:class>object.item.audioItem.musicTrack</upnp:class>", e(&m.title));
    if !m.artist.is_empty() {
        item.push_str(&format!("<dc:creator>{0}</dc:creator><upnp:artist>{0}</upnp:artist>", e(&m.artist)));
    }
    if let Some(a) = &m.album {
        item.push_str(&format!("<upnp:album>{}</upnp:album>", e(a)));
    }
    if let Some(a) = &m.art {
        item.push_str(&format!("<upnp:albumArtURI>{}</upnp:albumArtURI>", e(a)));
    }
    let dur = m.duration.map(|d| format!(" duration=\"{}.000\"", format_time(d))).unwrap_or_default();
    let flags = "DLNA.ORG_OP=01;DLNA.ORG_CI=0;DLNA.ORG_FLAGS=01700000000000000000000000000000";
    item.push_str(&format!("<res protocolInfo=\"http-get:*:{}:{flags}\"{dur}>{}</res>", e(&m.mime), e(uri)));
    format!(
        "<DIDL-Lite xmlns=\"urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:upnp=\"urn:schemas-upnp-org:metadata-1-0/upnp/\" xmlns:r=\"urn:schemas-rinconnetworks-com:metadata-1-0/\"><item id=\"glyph-0\" parentID=\"-1\" restricted=\"1\">{item}</item></DIDL-Lite>"
    )
}

/// The URI Sonos gets: MP3 live streams (no length) as radio, the rest as is.
pub fn sonos_uri(url: &str, mime: &str, live: bool) -> String {
    match url.strip_prefix("http://") {
        Some(rest) if live && mime == "audio/mpeg" => format!("x-rincon-mp3radio://{rest}"),
        _ => url.to_string(),
    }
}

/// Sonos group: (coordinator UUID, members as (UUID, description location)).
#[derive(Debug, Clone, PartialEq)]
pub struct ZoneGroup {
    pub coordinator: String,
    pub members: Vec<(String, String)>,
}

/// Parses `ZoneGroupState` (the inner XML, already unescaped).
pub fn parse_zone_groups(xml: &str) -> Vec<ZoneGroup> {
    let Ok(doc) = roxmltree::Document::parse(xml) else { return Vec::new() };
    doc.descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "ZoneGroup")
        .map(|g| ZoneGroup {
            coordinator: g.attribute("Coordinator").unwrap_or_default().to_string(),
            members: g
                .children()
                .filter(|m| m.is_element() && m.tag_name().name() == "ZoneGroupMember")
                .map(|m| (m.attribute("UUID").unwrap_or_default().to_string(), m.attribute("Location").unwrap_or_default().to_string()))
                .collect(),
        })
        .collect()
}

/// The AVTransport control URL of the group coordinator for `udn`.
fn sonos_coordinator(topology: &str, udn: &str) -> Option<String> {
    let r = soap(topology, ZONE_GROUP_TOPOLOGY, "GetZoneGroupState", &[]).ok()?;
    let state = xml_value(&r, "ZoneGroupState")?;
    let me = udn.trim_start_matches("uuid:");
    let me = me.split("_MR").next().unwrap_or(me);
    let g = parse_zone_groups(&state).into_iter().find(|g| g.members.iter().any(|(u, _)| u == me))?;
    if g.coordinator == me {
        return None;
    }
    let (_, loc) = g.members.iter().find(|(u, _)| *u == g.coordinator)?;
    Some(format!("{}/MediaRenderer/AVTransport/Control", http::Url::parse(loc)?.origin()))
}

// ------------------------------------------------------------ session

/// A connected renderer.
pub struct Renderer {
    av: String,
    rc: Option<String>,
    sonos: bool,
    /// When the last song was handed over; "stopped" right after that means
    /// still loading, not "song ended".
    loaded_at: Option<Instant>,
    played: bool,
    seek_after_start: Option<f64>,
    last_volume: Option<u8>,
    polls: u32,
}

impl Renderer {
    pub fn connect(d: &super::Device) -> std::io::Result<Self> {
        let desc = d.upnp.as_ref().ok_or_else(|| std::io::Error::other("not a UPnP device"))?;
        let mut av = desc.av_transport.clone().ok_or_else(|| std::io::Error::other("no AVTransport"))?;
        let sonos = desc.is_sonos();
        if sonos {
            if let Some(c) = desc.zone_group_topology.as_deref().and_then(|t| sonos_coordinator(t, &desc.udn)) {
                tracing::info!(coordinator = %c, "Sonos: playing through the group coordinator");
                av = c;
            }
        }
        // Reachable at all?
        soap(&av, AV_TRANSPORT, "GetTransportInfo", &[("InstanceID", "0")])?;
        Ok(Self { av, rc: desc.rendering_control.clone(), sonos, loaded_at: None, played: false, seek_after_start: None, last_volume: None, polls: 0 })
    }

    fn transport(&self, action: &str, extra: &[(&str, &str)]) -> std::io::Result<String> {
        let mut args = vec![("InstanceID", "0")];
        args.extend_from_slice(extra);
        soap(&self.av, AV_TRANSPORT, action, &args)
    }
}

impl Session for Renderer {
    fn load(&mut self, m: &Media, start: bool) -> std::io::Result<()> {
        let uri = if self.sonos { sonos_uri(&m.url, &m.mime, m.duration.is_none()) } else { m.url.clone() };
        let meta = didl(m, &uri);
        // From here on "stopped" means loading, later "could not play".
        self.loaded_at = Some(Instant::now());
        // Loaded paused: "stopped" is expected until the user presses play.
        self.played = !start;
        self.seek_after_start = (m.start_at > 1.0).then_some(m.start_at);
        // Some renderers refuse a new URI while playing.
        if !self.sonos {
            let _ = self.transport("Stop", &[]);
        }
        self.transport("SetAVTransportURI", &[("CurrentURI", &uri), ("CurrentURIMetaData", &meta)])?;
        if start {
            self.transport("Play", &[("Speed", "1")])?;
        }
        Ok(())
    }

    fn pause(&mut self, pause: bool) -> std::io::Result<()> {
        if pause {
            self.transport("Pause", &[]).map(|_| ())
        } else {
            self.transport("Play", &[("Speed", "1")]).map(|_| ())
        }
    }

    fn seek(&mut self, secs: f64) -> std::io::Result<()> {
        let t = format_time(secs);
        self.transport("Seek", &[("Unit", "REL_TIME"), ("Target", &t)]).or_else(|_| self.transport("Seek", &[("Unit", "ABS_TIME"), ("Target", &t)])).map(|_| ())
    }

    fn volume(&mut self, v: u8) -> std::io::Result<()> {
        let Some(rc) = &self.rc else { return Ok(()) };
        let vs = v.to_string();
        soap(rc, RENDERING_CONTROL, "SetVolume", &[("InstanceID", "0"), ("Channel", "Master"), ("DesiredVolume", &vs)])?;
        self.last_volume = Some(v);
        Ok(())
    }

    fn stop(&mut self) -> std::io::Result<()> {
        self.transport("Stop", &[]).map(|_| ())
    }

    fn status(&mut self) -> std::io::Result<Status> {
        let info = self.transport("GetTransportInfo", &[])?;
        let ts = xml_value(&info, "CurrentTransportState").unwrap_or_default();
        let pos = self.transport("GetPositionInfo", &[])?;
        let elapsed = xml_value(&pos, "RelTime").and_then(|t| parse_time(&t));
        let duration = xml_value(&pos, "TrackDuration").and_then(|t| parse_time(&t)).filter(|d| *d > 0.0);
        // Volume changes on the device itself are rare: ask every few polls.
        if self.polls % 4 == 0 {
            if let Some(rc) = &self.rc {
                if let Ok(r) = soap(rc, RENDERING_CONTROL, "GetVolume", &[("InstanceID", "0"), ("Channel", "Master")]) {
                    self.last_volume = xml_value(&r, "CurrentVolume").and_then(|v| v.trim().parse::<u32>().ok()).map(|v| v.min(100) as u8);
                }
            }
        }
        self.polls = self.polls.wrapping_add(1);
        let loading = self.loaded_at.is_some_and(|t| t.elapsed() < Duration::from_secs(10));
        let state = match ts.as_str() {
            "PLAYING" => {
                self.played = true;
                if let Some(s) = self.seek_after_start.take() {
                    let _ = self.seek(s);
                }
                "play"
            }
            "PAUSED_PLAYBACK" | "PAUSED_RECORDING" => "pause",
            "TRANSITIONING" => "load",
            _ if loading && !self.played => "load",
            // Loaded but never played: the device could not open the song.
            _ if self.loaded_at.is_some() && !self.played => "error",
            _ => "stop",
        };
        Ok(Status { state: state.to_string(), elapsed, duration, volume: self.last_volume })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TV: &str = r#"<?xml version="1.0"?>
<root xmlns="urn:schemas-upnp-org:device-1-0" xmlns:dlna="urn:schemas-dlna-org:device-1-0">
  <specVersion><major>1</major><minor>0</minor></specVersion>
  <device>
    <deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType>
    <friendlyName>[TV] Samsung 7 Series (55)</friendlyName>
    <manufacturer>Samsung Electronics</manufacturer>
    <modelName>UE55RU7179</modelName>
    <UDN>uuid:0815-tv</UDN>
    <serviceList>
      <service>
        <serviceType>urn:schemas-upnp-org:service:RenderingControl:1</serviceType>
        <serviceId>urn:upnp-org:serviceId:RenderingControl</serviceId>
        <controlURL>/upnp/control/RenderingControl1</controlURL>
      </service>
      <service>
        <serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType>
        <serviceId>urn:upnp-org:serviceId:AVTransport</serviceId>
        <controlURL>control/AVTransport1</controlURL>
      </service>
    </serviceList>
  </device>
</root>"#;

    const SONOS: &str = r#"<?xml version="1.0" encoding="utf-8" ?>
<root xmlns="urn:schemas-upnp-org:device-1-0">
  <specVersion><major>1</major><minor>0</minor></specVersion>
  <device>
    <deviceType>urn:schemas-upnp-org:device:ZonePlayer:1</deviceType>
    <friendlyName>192.168.1.30 - Sonos One - RINCON_48A6B8C1D2E301400</friendlyName>
    <manufacturer>Sonos, Inc.</manufacturer>
    <modelName>Sonos One</modelName>
    <UDN>uuid:RINCON_48A6B8C1D2E301400</UDN>
    <roomName>Küche</roomName>
    <serviceList>
      <service>
        <serviceType>urn:schemas-upnp-org:service:ZoneGroupTopology:1</serviceType>
        <controlURL>/ZoneGroupTopology/Control</controlURL>
      </service>
    </serviceList>
    <deviceList>
      <device>
        <deviceType>urn:schemas-upnp-org:device:MediaServer:1</deviceType>
        <UDN>uuid:RINCON_48A6B8C1D2E301400_MS</UDN>
        <serviceList><service>
          <serviceType>urn:schemas-upnp-org:service:ContentDirectory:1</serviceType>
          <controlURL>/MediaServer/ContentDirectory/Control</controlURL>
        </service></serviceList>
      </device>
      <device>
        <deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType>
        <friendlyName>Küche - Sonos One Media Renderer</friendlyName>
        <UDN>uuid:RINCON_48A6B8C1D2E301400_MR</UDN>
        <serviceList>
          <service>
            <serviceType>urn:schemas-upnp-org:service:RenderingControl:1</serviceType>
            <controlURL>/MediaRenderer/RenderingControl/Control</controlURL>
          </service>
          <service>
            <serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType>
            <controlURL>/MediaRenderer/AVTransport/Control</controlURL>
          </service>
        </serviceList>
      </device>
    </deviceList>
  </device>
</root>"#;

    #[test]
    fn parses_a_tv() {
        let d = parse_description(TV, "http://10.0.0.5:9197/dmr").unwrap();
        assert_eq!(d.friendly_name, "[TV] Samsung 7 Series (55)");
        assert_eq!(d.av_transport.as_deref(), Some("http://10.0.0.5:9197/control/AVTransport1"));
        assert_eq!(d.rendering_control.as_deref(), Some("http://10.0.0.5:9197/upnp/control/RenderingControl1"));
        assert!(!d.is_sonos());
        assert_eq!(d.udn, "uuid:0815-tv");
    }

    #[test]
    fn parses_sonos() {
        let d = parse_description(SONOS, "http://192.168.1.30:1400/xml/device_description.xml").unwrap();
        assert_eq!(d.friendly_name, "Küche");
        assert!(d.is_sonos());
        assert_eq!(d.av_transport.as_deref(), Some("http://192.168.1.30:1400/MediaRenderer/AVTransport/Control"));
        assert_eq!(d.zone_group_topology.as_deref(), Some("http://192.168.1.30:1400/ZoneGroupTopology/Control"));
        assert_eq!(d.udn, "uuid:RINCON_48A6B8C1D2E301400");
        assert!(parse_description("<root>", "http://x/").is_none());
    }

    #[test]
    fn zone_groups() {
        let xml = r#"<ZoneGroupState><ZoneGroups><ZoneGroup Coordinator="RINCON_B" ID="RINCON_B:12"><ZoneGroupMember UUID="RINCON_A" Location="http://192.168.1.30:1400/xml/device_description.xml" ZoneName="Küche"/><ZoneGroupMember UUID="RINCON_B" Location="http://192.168.1.31:1400/xml/device_description.xml" ZoneName="Wohnzimmer"><Satellite UUID="RINCON_C" Location="http://192.168.1.32:1400/xml/device_description.xml"/></ZoneGroupMember></ZoneGroup></ZoneGroups></ZoneGroupState>"#;
        let g = parse_zone_groups(xml);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].coordinator, "RINCON_B");
        assert_eq!(g[0].members.len(), 2);
        assert_eq!(g[0].members[1].1, "http://192.168.1.31:1400/xml/device_description.xml");
    }

    #[test]
    fn soap_and_values() {
        let b = soap_body(AV_TRANSPORT, "SetAVTransportURI", &[("InstanceID", "0"), ("CurrentURI", "http://h/a?b=1&c=<2>")]);
        assert!(b.contains("<u:SetAVTransportURI xmlns:u=\"urn:schemas-upnp-org:service:AVTransport:1\">"));
        assert!(b.contains("<CurrentURI>http://h/a?b=1&amp;c=&lt;2&gt;</CurrentURI>"));
        let resp = r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><u:GetPositionInfoResponse xmlns:u="urn:schemas-upnp-org:service:AVTransport:1"><Track>1</Track><TrackDuration>0:03:25</TrackDuration><RelTime>0:01:02.500</RelTime></u:GetPositionInfoResponse></s:Body></s:Envelope>"#;
        assert_eq!(xml_value(resp, "TrackDuration").and_then(|t| parse_time(&t)), Some(205.0));
        assert_eq!(xml_value(resp, "RelTime").and_then(|t| parse_time(&t)), Some(62.5));
        assert_eq!(parse_time("NOT_IMPLEMENTED"), None);
        assert_eq!(format_time(3725.4), "1:02:05");
    }

    #[test]
    fn metadata() {
        let m = Media { url: "http://10.0.0.2:4000/x.mp3".into(), mime: "audio/mpeg".into(), title: "Rock & Roll".into(), artist: "A".into(), album: None, art: None, duration: Some(61.0), start_at: 0.0 };
        let d = didl(&m, &m.url);
        assert!(d.contains("<dc:title>Rock &amp; Roll</dc:title>"));
        assert!(d.contains("protocolInfo=\"http-get:*:audio/mpeg:DLNA.ORG_OP=01"));
        assert!(d.contains("duration=\"0:01:01.000\""));
        assert!(roxmltree::Document::parse(&d).is_ok());
        assert_eq!(sonos_uri("http://10.0.0.2:4000/live.mp3", "audio/mpeg", true), "x-rincon-mp3radio://10.0.0.2:4000/live.mp3");
        assert_eq!(sonos_uri("http://10.0.0.2:4000/x.m4a", "audio/mp4", true), "http://10.0.0.2:4000/x.m4a");
        assert_eq!(sonos_uri("http://10.0.0.2:4000/x.mp3", "audio/mpeg", false), "http://10.0.0.2:4000/x.mp3");
    }
}
