//! Deny by default: a plugin never gets a raw backend, MCP, account or server API.
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub fn known_function(f: &str) -> bool {
    matches!(f,
        "app.info" | "app.text" | "app.now" | "ui.get" | "ui.values" | "ui.set" | "ui.update" |
        "ui.toast" | "ui.open" | "ui.search" | "ui.open_artist" |
        "storage.get" | "storage.keys" | "storage.set" | "storage.delete" | "storage.clear" |
        "timer.set" | "timer.clear" | "net.fetch" |
        "player.state" | "player.play" | "player.pause" | "player.toggle" | "player.next" |
        "player.previous" | "player.seek" | "player.volume" | "player.shuffle" | "player.repeat" |
        "player.play_song" | "player.play_artist" | "queue.get" | "queue.add" | "queue.remove" | "queue.move" |
        "search" | "library.playlists" | "library.playlist" | "library.play_playlist" |
        "library.create_playlist" | "library.rename_playlist" | "library.delete_playlist" |
        "library.add_to_playlist" | "library.remove_from_playlist" | "library.like" | "library.unlike" |
        "lyrics.get" | "lyrics.publish" | "stats.get" | "settings.get" | "settings.set" |
        "audio.spectrum" | "audio.equalizer" | "audio.set_equalizer")
}

/// Service calls can consume server/provider quota even when initiated through UI.
fn service_call(f: &str) -> bool {
    matches!(f, "search" | "ui.search" | "ui.open_artist" | "player.play_song" | "player.play_artist" | "queue.add")
}

#[derive(Default)]
pub struct CallBudget {
    all: VecDeque<Instant>,
    services: VecDeque<Instant>,
    network: VecDeque<Instant>,
}

impl CallBudget {
    pub fn admit(&mut self, func: &str) -> bool {
        let now = Instant::now();
        fn trim(q: &mut VecDeque<Instant>, now: Instant, window: Duration) {
            while q.front().is_some_and(|t| now.duration_since(*t) >= window) { q.pop_front(); }
        }
        trim(&mut self.all, now, Duration::from_secs(1));
        trim(&mut self.services, now, Duration::from_secs(60));
        trim(&mut self.network, now, Duration::from_secs(60));
        if self.all.len() >= 30 || service_call(func) && self.services.len() >= 12
            || func == "net.fetch" && self.network.len() >= 30 { return false; }
        self.all.push_back(now);
        if service_call(func) { self.services.push_back(now); }
        if func == "net.fetch" { self.network.push_back(now); }
        true
    }
}

/// Only globally routed addresses. DNS results are checked and pinned per request.
pub fn public_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => {
            let [a,b,c,_] = ip.octets();
            !(ip.is_private() || ip.is_loopback() || ip.is_link_local() || a == 0 || a >= 224
                || a == 100 && (64..=127).contains(&b) || a == 169 && b == 254
                || a == 192 && b == 0 || a == 192 && b == 168
                || a == 198 && (b == 18 || b == 19 || b == 51 && c == 100)
                || a == 203 && b == 0 && c == 113)
        }
        std::net::IpAddr::V6(ip) => {
            let s = ip.segments();
            // Avoid mapped IPv4, local, multicast and transition mechanisms.
            s[0] & 0xe000 == 0x2000 && s[0] != 0x2002
                && !(s[0] == 0x2001 && (s[1] < 0x200 || s[1] == 0xdb8))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_server_and_internal_calls_are_never_exposed() {
        for f in ["server.fetch", "mcp.call", "account.token", "host.stopped", "admin.config", "net.raw", "audio.raw"] {
            assert!(!known_function(f), "{f}");
        }
        assert!(known_function("audio.spectrum"));
        assert!(known_function("lyrics.publish"));
    }
    #[test]
    fn private_and_metadata_destinations_are_blocked() {
        for ip in ["127.0.0.1", "10.0.0.1", "169.254.169.254", "100.100.100.200", "192.168.1.1", "::1", "::ffff:127.0.0.1", "fc00::1", "2001:db8::1", "224.0.0.1"] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] { assert!(public_ip(ip.parse().unwrap())); }
    }
    #[test]
    fn ui_paths_share_the_service_budget() {
        let mut b = CallBudget::default();
        for _ in 0..6 { assert!(b.admit("search")); assert!(b.admit("ui.search")); }
        assert!(!b.admit("player.play_song"));
        assert!(!b.admit("ui.open_artist"));
        assert!(b.admit("player.pause"));
    }
}
