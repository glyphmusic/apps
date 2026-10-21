//! Single instance: the first Glyph process listens on a local port; a second
//! launch (e.g. a click in the Windows taskbar jump list, which starts
//! `glyph --dock-action N`) hands its request to the running app and exits.
//!
//! Messages are single lines: `activate` or `dock <index>`.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Mutex;
use std::time::Duration;

/// Loopback only; never reachable from other machines.
const ADDR: &str = "127.0.0.1:47613";

static INBOX: Mutex<Option<flume::Receiver<String>>> = Mutex::new(None);
/// Links the system handed to this process directly (macOS Apple Events,
/// Android intents), waiting for the app.
static OPENED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The request a `glyph://` link (or a jam / playlist web link) stands for.
pub fn request_for_url(url: &str) -> Option<String> {
    if url.starts_with(glyph_protocol::jam::APP_LINK_PREFIX) || glyph_protocol::jam::is_jam_text(url) {
        // `glyph://jam/<code>` (the jam invite page and QR code).
        return glyph_protocol::jam::normalize_code(url).map(|code| format!("jam {code}"));
    }
    glyph_core::share::link_id(url).map(|id| format!("open https://glyph.my/p/{id}"))
}

/// A link the system opened Glyph with while it runs (macOS hands URLs to
/// the running app, Android to the activity); the app picks it up like a
/// request of a second launch.
pub fn open_url(url: &str) {
    if let Some(req) = request_for_url(url) {
        if let Ok(mut v) = OPENED.lock() {
            v.push(req);
        }
    }
}

/// What this launch asks for, from the command line.
pub fn request_from_args() -> String {
    let args: Vec<String> = std::env::args().collect();
    if let Some(req) = args.iter().skip(1).find_map(|a| request_for_url(a)) {
        return req;
    }
    match args.iter().position(|a| a == "--dock-action").and_then(|i| args.get(i + 1)) {
        Some(n) if n.parse::<usize>().is_ok() => format!("dock {n}"),
        _ => "activate".into(),
    }
}

/// Becomes the primary instance (true), or forwards `request` to the running
/// one (false: this process should exit).
pub fn claim_or_forward(request: &str) -> bool {
    match TcpListener::bind(ADDR) {
        Ok(listener) => {
            let (tx, rx) = flume::unbounded();
            *INBOX.lock().unwrap() = Some(rx);
            // A dock action that started the app runs once it is up.
            if request != "activate" {
                let _ = tx.send(request.to_string());
            }
            std::thread::Builder::new()
                .name("glyph-instance".into())
                .spawn(move || {
                    for stream in listener.incoming().flatten() {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let mut line = String::new();
                        if BufReader::new(stream).read_line(&mut line).is_ok() {
                            let line = line.trim();
                            if line == "activate" || line.starts_with("dock ") || line.strip_prefix("jam ").is_some_and(|c| glyph_protocol::jam::normalize_code(c).as_deref() == Some(c)) || line.strip_prefix("open ").is_some_and(|s| glyph_core::share::link_id(s).is_some()) {
                                let _ = tx.send(line.to_string());
                            }
                        }
                    }
                })
                .ok();
            true
        }
        Err(_) => match TcpStream::connect_timeout(&ADDR.parse().expect("addr"), Duration::from_millis(500)) {
            Ok(mut s) => {
                let _ = writeln!(s, "{request}");
                false
            }
            // Port taken by something else: just run normally.
            Err(_) => true,
        },
    }
}

/// Next request from another launch or an opened link, if any (polled by
/// the app).
pub fn try_recv() -> Option<String> {
    if let Some(req) = OPENED.lock().ok().and_then(|mut v| (!v.is_empty()).then(|| v.remove(0))) {
        return Some(req);
    }
    INBOX.lock().ok()?.as_ref()?.try_recv().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_become_requests() {
        assert_eq!(request_for_url("glyph://jam/k7p2qx").as_deref(), Some("jam K7P2QX"));
        assert_eq!(request_for_url("https://glyph.my/jam/K7P2QX").as_deref(), Some("jam K7P2QX"));
        assert_eq!(request_for_url("glyph://jam/nope"), None);
        assert_eq!(request_for_url("glyph://p/AbC123xy").as_deref(), Some("open https://glyph.my/p/AbC123xy"));
        assert_eq!(request_for_url("https://example.com"), None);
        open_url("glyph://jam/K7P2QX");
        assert_eq!(try_recv().as_deref(), Some("jam K7P2QX"));
    }

    #[test]
    fn request_defaults_to_activate() {
        assert_eq!(request_from_args(), "activate");
    }
}
