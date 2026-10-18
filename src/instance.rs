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

/// What this launch asks for, from the command line.
pub fn request_from_args() -> String {
    let args: Vec<String> = std::env::args().collect();
    if let Some(link) = args.iter().skip(1).find(|s| glyph_core::share::link_id(s).is_some()) {
        let id = glyph_core::share::link_id(link).unwrap();
        return format!("open https://glyph.my/p/{id}");
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
                            if line == "activate" || line.starts_with("dock ") || line.strip_prefix("open ").is_some_and(|s| glyph_core::share::link_id(s).is_some()) {
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

/// Next request from another launch, if any (polled by the app).
pub fn try_recv() -> Option<String> {
    INBOX.lock().ok()?.as_ref()?.try_recv().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_defaults_to_activate() {
        assert_eq!(request_from_args(), "activate");
    }
}
