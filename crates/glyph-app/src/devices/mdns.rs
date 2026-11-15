//! A small mDNS / DNS-SD browser (RFC 6762 / 6763) in plain std: it sends
//! "legacy unicast" queries (from a random port, so responders answer
//! straight back to us) and reads PTR / SRV / TXT / A / AAAA records.
//! No system service (Bonjour, Avahi) is needed and port 5353 stays free
//! for them.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

const GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
const PORT: u16 = 5353;

pub const TYPE_A: u16 = 1;
pub const TYPE_PTR: u16 = 12;
pub const TYPE_TXT: u16 = 16;
pub const TYPE_AAAA: u16 = 28;
pub const TYPE_SRV: u16 = 33;

/// Record data Glyph cares about.
#[derive(Debug, Clone, PartialEq)]
pub enum RData {
    Ptr(String),
    Srv { port: u16, target: String },
    Txt(Vec<(String, String)>),
    A(Ipv4Addr),
    Aaaa(Ipv6Addr),
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// Lowercase, without the trailing dot. Dots inside a label are `\.`.
    pub name: String,
    pub rtype: u16,
    pub data: RData,
}

/// One found service instance.
#[derive(Debug, Clone, PartialEq)]
pub struct Service {
    /// Instance label as the device names itself ("Living Room").
    pub label: String,
    /// `_googlecast._tcp.local`
    pub service_type: String,
    pub host: String,
    pub port: u16,
    pub txt: Vec<(String, String)>,
    pub addrs: Vec<IpAddr>,
}

impl Service {
    pub fn txt(&self, key: &str) -> Option<&str> {
        self.txt.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }

    /// IPv4 first (what most devices answer on), else IPv6.
    pub fn addr(&self) -> Option<IpAddr> {
        self.addrs.iter().find(|a| a.is_ipv4()).or_else(|| self.addrs.first()).copied()
    }
}

// ------------------------------------------------------------ packets

fn encode_name(out: &mut Vec<u8>, name: &str) {
    for label in name.trim_end_matches('.').split('.').filter(|l| !l.is_empty()) {
        let b = label.as_bytes();
        out.push(b.len().min(63) as u8);
        out.extend_from_slice(&b[..b.len().min(63)]);
    }
    out.push(0);
}

/// A query packet asking `qtype` for each name.
pub fn query(names: &[(&str, u16)]) -> Vec<u8> {
    let mut p = Vec::with_capacity(64);
    // id, flags (standard query), qd, an, ns, ar
    p.extend_from_slice(&[0, 0, 0, 0]);
    p.extend_from_slice(&(names.len() as u16).to_be_bytes());
    p.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    for (n, t) in names {
        encode_name(&mut p, n);
        p.extend_from_slice(&t.to_be_bytes());
        p.extend_from_slice(&1u16.to_be_bytes()); // IN
    }
    p
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]))
}

/// Reads a (possibly compressed) name at `pos`; the name and the position
/// after it in the packet.
fn read_name(b: &[u8], mut pos: usize) -> Option<(String, usize)> {
    let mut labels: Vec<String> = Vec::new();
    let mut end = None;
    let mut jumps = 0;
    loop {
        let len = *b.get(pos)? as usize;
        if len & 0xC0 == 0xC0 {
            let ptr = (u16_at(b, pos)? & 0x3FFF) as usize;
            end.get_or_insert(pos + 2);
            jumps += 1;
            if jumps > 32 || ptr >= b.len() {
                return None;
            }
            pos = ptr;
            continue;
        }
        if len == 0 {
            end.get_or_insert(pos + 1);
            break;
        }
        let label = b.get(pos + 1..pos + 1 + len)?;
        labels.push(String::from_utf8_lossy(label).replace('.', "\\."));
        pos += 1 + len;
        if labels.len() > 64 {
            return None;
        }
    }
    Some((labels.join(".").to_lowercase(), end?))
}

/// The display form of an instance label (case kept): its first label.
fn first_label(b: &[u8], pos: usize) -> Option<String> {
    // The instance label as sent (not lowercased), for display.
    let mut pos = pos;
    for _ in 0..32 {
        let len = *b.get(pos)? as usize;
        if len & 0xC0 == 0xC0 {
            pos = (u16_at(b, pos)? & 0x3FFF) as usize;
            continue;
        }
        return Some(String::from_utf8_lossy(b.get(pos + 1..pos + 1 + len)?).into_owned());
    }
    None
}

fn parse_txt(d: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < d.len() {
        let n = d[i] as usize;
        let Some(s) = d.get(i + 1..i + 1 + n) else { break };
        let s = String::from_utf8_lossy(s);
        if !s.is_empty() {
            match s.split_once('=') {
                Some((k, v)) => out.push((k.to_string(), v.to_string())),
                None => out.push((s.to_string(), String::new())),
            }
        }
        i += 1 + n;
    }
    out
}

/// Every resource record of a response packet (answers, authority,
/// additional), plus the display labels of PTR targets (name -> label).
pub fn parse(b: &[u8]) -> Option<(Vec<Record>, HashMap<String, String>)> {
    let flags = u16_at(b, 2)?;
    if flags & 0x8000 == 0 {
        return None; // a query, not a response
    }
    let qd = u16_at(b, 4)? as usize;
    let rr = u16_at(b, 6)? as usize + u16_at(b, 8)? as usize + u16_at(b, 10)? as usize;
    let mut pos = 12;
    for _ in 0..qd {
        pos = read_name(b, pos)?.1 + 4;
    }
    let mut out = Vec::new();
    let mut labels = HashMap::new();
    for _ in 0..rr {
        let (name, p) = read_name(b, pos)?;
        let rtype = u16_at(b, p)?;
        let rdlen = u16_at(b, p + 8)? as usize;
        let start = p + 10;
        let d = b.get(start..start + rdlen)?;
        let data = match rtype {
            TYPE_PTR => {
                let target = read_name(b, start)?.0;
                if let Some(l) = first_label(b, start) {
                    labels.insert(target.clone(), l);
                }
                RData::Ptr(target)
            }
            TYPE_SRV if rdlen >= 7 => RData::Srv { port: u16_at(d, 4)?, target: read_name(b, start + 6)?.0 },
            TYPE_TXT => RData::Txt(parse_txt(d)),
            TYPE_A if rdlen == 4 => RData::A(Ipv4Addr::new(d[0], d[1], d[2], d[3])),
            TYPE_AAAA if rdlen == 16 => {
                let mut a = [0u8; 16];
                a.copy_from_slice(d);
                RData::Aaaa(Ipv6Addr::from(a))
            }
            _ => RData::Other,
        };
        out.push(Record { name, rtype, data });
        pos = start + rdlen;
    }
    Some((out, labels))
}

/// Instances of `service_type` found in `records` (those whose SRV is known).
pub fn collect(records: &[Record], labels: &HashMap<String, String>, service_type: &str) -> Vec<Service> {
    let st = service_type.trim_end_matches('.').to_lowercase();
    let mut out: Vec<Service> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for r in records {
        let RData::Ptr(instance) = &r.data else { continue };
        if r.name != st || seen.contains(&instance.as_str()) {
            continue;
        }
        seen.push(instance);
        let Some((port, host)) = records.iter().find_map(|x| match &x.data {
            RData::Srv { port, target } if &x.name == instance => Some((*port, target.clone())),
            _ => None,
        }) else {
            continue;
        };
        let txt = records
            .iter()
            .find_map(|x| match &x.data {
                RData::Txt(t) if &x.name == instance => Some(t.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let mut addrs: Vec<IpAddr> = Vec::new();
        for x in records.iter().filter(|x| x.name == host) {
            let a = match x.data {
                RData::A(a) => IpAddr::V4(a),
                RData::Aaaa(a) => IpAddr::V6(a),
                _ => continue,
            };
            if !addrs.contains(&a) {
                addrs.push(a);
            }
        }
        let label = labels.get(instance).cloned().unwrap_or_else(|| {
            instance.strip_suffix(&format!(".{st}")).unwrap_or(instance).replace("\\.", ".")
        });
        out.push(Service { label, service_type: st.clone(), host, port, txt, addrs });
    }
    out
}

// ------------------------------------------------------------ browsing

fn listen(socks: &[UdpSocket], until: Instant, records: &mut Vec<Record>, labels: &mut HashMap<String, String>, from: &mut HashMap<String, IpAddr>) {
    super::recv_all(socks, until, |b, src| {
        if let Some((r, l)) = parse(b) {
            for x in &r {
                if let RData::Srv { target, .. } = &x.data {
                    from.entry(target.clone()).or_insert(src.ip());
                }
            }
            records.extend(r);
            labels.extend(l);
        }
    });
}

fn send_all(socks: &[UdpSocket], packet: &[u8]) {
    let dest = SocketAddr::from((GROUP, PORT));
    for s in socks {
        let _ = s.send_to(packet, dest);
    }
}

/// Asks the network for `service_types` (e.g. `_googlecast._tcp.local`) on
/// every network adapter and collects answers for about `wait`.
pub fn browse(service_types: &[&str], wait: Duration) -> Vec<Service> {
    let socks = super::lan_sockets(255);
    if socks.is_empty() {
        return Vec::new();
    }
    let q: Vec<(&str, u16)> = service_types.iter().map(|s| (*s, TYPE_PTR)).collect();
    let packet = query(&q);
    let (mut records, mut labels, mut from) = (Vec::new(), HashMap::new(), HashMap::new());
    let start = Instant::now();
    send_all(&socks, &packet);
    listen(&socks, start + wait / 3, &mut records, &mut labels, &mut from);
    // Repeat once: multicast gets lost on busy Wi-Fi.
    send_all(&socks, &packet);
    listen(&socks, start + wait * 2 / 3, &mut records, &mut labels, &mut from);

    // Instances without SRV / TXT and hosts without address: ask directly.
    let mut follow: Vec<(String, u16)> = Vec::new();
    for r in &records {
        if let RData::Ptr(inst) = &r.data {
            if service_types.iter().any(|s| s.trim_end_matches('.').eq_ignore_ascii_case(&r.name)) && !records.iter().any(|x| &x.name == inst && x.rtype == TYPE_SRV) {
                follow.push((inst.clone(), TYPE_SRV));
                follow.push((inst.clone(), TYPE_TXT));
            }
        }
        if let RData::Srv { target, .. } = &r.data {
            if !records.iter().any(|x| &x.name == target && matches!(x.data, RData::A(_) | RData::Aaaa(_))) {
                follow.push((target.clone(), TYPE_A));
            }
        }
    }
    follow.sort();
    follow.dedup();
    if !follow.is_empty() {
        let refs: Vec<(&str, u16)> = follow.iter().take(24).map(|(n, t)| (n.as_str(), *t)).collect();
        send_all(&socks, &query(&refs));
        listen(&socks, Instant::now() + wait / 3, &mut records, &mut labels, &mut from);
    }

    let mut out = Vec::new();
    for st in service_types {
        for mut s in collect(&records, &labels, st) {
            // No address record at all: the packet's sender is the device.
            if s.addrs.is_empty() {
                if let Some(ip) = from.get(&s.host) {
                    s.addrs.push(*ip);
                }
            }
            out.push(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(out: &mut Vec<u8>, n: &str) {
        encode_name(out, n)
    }

    fn rr(out: &mut Vec<u8>, n: &[u8], rtype: u16, data: &[u8]) {
        out.extend_from_slice(n);
        out.extend_from_slice(&rtype.to_be_bytes());
        out.extend_from_slice(&[0x80, 0x01, 0, 0, 0x11, 0x94]);
        out.extend_from_slice(&(data.len() as u16).to_be_bytes());
        out.extend_from_slice(data);
    }

    /// A Chromecast-style answer: PTR in answers, SRV / TXT / A additional,
    /// with name compression.
    fn cast_response() -> Vec<u8> {
        let mut p = vec![0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 3];
        // answer: _googlecast._tcp.local PTR "Wohnzimmer TV._googlecast._tcp.local"
        let st_off = p.len();
        name(&mut p, "_googlecast._tcp.local");
        p.extend_from_slice(&TYPE_PTR.to_be_bytes());
        p.extend_from_slice(&[0, 1, 0, 0, 0x11, 0x94]);
        let mut target = vec![13];
        target.extend_from_slice(b"Wohnzimmer TV");
        target.extend_from_slice(&[0xC0, st_off as u8]);
        p.extend_from_slice(&(target.len() as u16).to_be_bytes());
        let inst_off = p.len();
        p.extend_from_slice(&target);
        // SRV: port 8009, target abc.local
        let mut srv = vec![0, 0, 0, 0, 0x1F, 0x49];
        name(&mut srv, "abc.local");
        rr(&mut p, &[0xC0, inst_off as u8], TYPE_SRV, &srv);
        // TXT
        let mut txt = Vec::new();
        for kv in ["id=1234abcd", "md=Chromecast", "fn=Wohnzimmer TV"] {
            txt.push(kv.len() as u8);
            txt.extend_from_slice(kv.as_bytes());
        }
        rr(&mut p, &[0xC0, inst_off as u8], TYPE_TXT, &txt);
        let mut host = Vec::new();
        name(&mut host, "abc.local");
        rr(&mut p, &host, TYPE_A, &[192, 168, 1, 23]);
        p
    }

    #[test]
    fn builds_queries() {
        let q = query(&[("_mpd._tcp.local", TYPE_PTR)]);
        assert_eq!(&q[4..6], &[0, 1]);
        assert_eq!(&q[12..17], b"\x04_mpd");
        assert_eq!(&q[q.len() - 4..], &[0, 12, 0, 1]);
        assert_eq!(read_name(&q, 12).unwrap().0, "_mpd._tcp.local");
    }

    #[test]
    fn parses_records_with_compression() {
        let p = cast_response();
        let (records, labels) = parse(&p).unwrap();
        assert_eq!(records.len(), 4);
        assert_eq!(records[0].data, RData::Ptr("wohnzimmer tv._googlecast._tcp.local".into()));
        assert_eq!(records[1].data, RData::Srv { port: 8009, target: "abc.local".into() });
        let s = collect(&records, &labels, "_googlecast._tcp.local.");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].label, "Wohnzimmer TV");
        assert_eq!(s[0].port, 8009);
        assert_eq!(s[0].txt("md"), Some("Chromecast"));
        assert_eq!(s[0].txt("FN"), Some("Wohnzimmer TV"));
        assert_eq!(s[0].addr(), Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 23))));
    }

    #[test]
    fn ignores_queries_and_garbage() {
        assert!(parse(&query(&[("_raop._tcp.local", TYPE_PTR)])).is_none());
        assert!(parse(&[0, 0, 0x84]).is_none());
        // A pointer loop must not hang.
        let p = [0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0xC0, 12];
        assert!(parse(&p).is_none());
        // Missing SRV: nothing to connect to yet.
        let mut p = cast_response();
        p[11] = 0; // drop the additional records
        let (r, l) = parse(&p).unwrap();
        assert!(collect(&r, &l, "_googlecast._tcp.local").is_empty());
    }

    #[test]
    fn txt_entries() {
        assert_eq!(parse_txt(b"\x05a=b=c\x04flag\x00"), vec![("a".into(), "b=c".into()), ("flag".into(), String::new())]);
    }
}
