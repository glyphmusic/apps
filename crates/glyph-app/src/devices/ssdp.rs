//! SSDP discovery (UPnP 1.x): one M-SEARCH to the multicast group, devices
//! answer by unicast with where their description lives.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

const GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
const PORT: u16 = 1900;

pub const MEDIA_RENDERER: &str = "urn:schemas-upnp-org:device:MediaRenderer:1";
pub const ZONE_PLAYER: &str = "urn:schemas-upnp-org:device:ZonePlayer:1";

/// One answer to an M-SEARCH.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub location: String,
    pub st: String,
    pub usn: String,
    pub server: String,
}

pub fn m_search(st: &str, mx: u8) -> String {
    format!(
        "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: {mx}\r\nST: {st}\r\nUSER-AGENT: {}/1.0 UPnP/1.1 Glyph/{}\r\n\r\n",
        std::env::consts::OS,
        env!("CARGO_PKG_VERSION")
    )
}

/// Parses a search response (or a NOTIFY alive message).
pub fn parse_response(s: &str) -> Option<Response> {
    let mut lines = s.split("\r\n").flat_map(|l| l.split('\n'));
    let first = lines.next()?.trim();
    let ok = first.starts_with("HTTP/1.1 200") || first.starts_with("HTTP/1.0 200") || first.starts_with("NOTIFY ");
    if !ok {
        return None;
    }
    let mut r = Response { location: String::new(), st: String::new(), usn: String::new(), server: String::new() };
    for l in lines {
        let Some((k, v)) = l.split_once(':') else { continue };
        let v = v.trim().to_string();
        match k.trim().to_ascii_lowercase().as_str() {
            "location" => r.location = v,
            "st" | "nt" => r.st = v,
            "usn" => r.usn = v,
            "server" => r.server = v,
            "nts" if v != "ssdp:alive" => return None,
            _ => {}
        }
    }
    (r.location.starts_with("http://")).then_some(r)
}

/// Searches for `targets` and collects answers for `wait` (one per location).
pub fn search(targets: &[&str], wait: Duration) -> Vec<Response> {
    let socks: Vec<UdpSocket> = super::lan_sockets(4);
    let dest = SocketAddr::from((GROUP, PORT));
    let mx = wait.as_secs().clamp(1, 5) as u8;
    let send = || {
        for s in &socks {
            for t in targets {
                let _ = s.send_to(m_search(t, mx).as_bytes(), dest);
            }
        }
    };
    let start = Instant::now();
    let mut out: Vec<Response> = Vec::new();
    let mut take = |b: &[u8], _: SocketAddr| {
        if let Some(r) = parse_response(&String::from_utf8_lossy(b)) {
            if !out.iter().any(|o| o.location == r.location) {
                out.push(r);
            }
        }
    };
    send();
    super::recv_all(&socks, start + wait / 3, &mut take);
    // UDP gets lost on Wi-Fi; UPnP asks to send searches more than once.
    send();
    super::recv_all(&socks, start + wait, &mut take);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_message() {
        let m = m_search(MEDIA_RENDERER, 2);
        assert!(m.starts_with("M-SEARCH * HTTP/1.1\r\n"));
        assert!(m.contains("MAN: \"ssdp:discover\"\r\n"));
        assert!(m.contains("ST: urn:schemas-upnp-org:device:MediaRenderer:1\r\n"));
        assert!(m.ends_with("\r\n\r\n"));
    }

    #[test]
    fn parses_answers() {
        let sonos = "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age = 1800\r\nEXT:\r\nLOCATION: http://192.168.1.30:1400/xml/device_description.xml\r\nSERVER: Linux UPnP/1.0 Sonos/70.3-35220 (ZPS9)\r\nST: urn:schemas-upnp-org:device:ZonePlayer:1\r\nUSN: uuid:RINCON_48A6B8C1D2E301400::urn:schemas-upnp-org:device:ZonePlayer:1\r\nX-RINCON-HOUSEHOLD: Sonos_abc\r\n\r\n";
        let r = parse_response(sonos).unwrap();
        assert_eq!(r.location, "http://192.168.1.30:1400/xml/device_description.xml");
        assert_eq!(r.st, ZONE_PLAYER);
        assert!(r.usn.starts_with("uuid:RINCON_"));
        assert!(r.server.contains("Sonos"));

        // Lowercase headers, bare \n line ends.
        let tv = "HTTP/1.1 200 OK\nlocation: http://10.0.0.5:52235/dmr/SamsungMRDesc.xml\nst: urn:schemas-upnp-org:device:MediaRenderer:1\nusn: uuid:0815\n\n";
        assert_eq!(parse_response(tv).unwrap().location, "http://10.0.0.5:52235/dmr/SamsungMRDesc.xml");

        let notify = "NOTIFY * HTTP/1.1\r\nLOCATION: http://10.0.0.7:49152/desc.xml\r\nNT: urn:schemas-upnp-org:device:MediaRenderer:1\r\nNTS: ssdp:byebye\r\n\r\n";
        assert_eq!(parse_response(notify), None);
        assert_eq!(parse_response("HTTP/1.1 404 Not Found\r\n\r\n"), None);
        assert_eq!(parse_response("HTTP/1.1 200 OK\r\nST: x\r\n\r\n"), None);
    }
}
