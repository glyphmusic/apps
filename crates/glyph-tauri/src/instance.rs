//! Single instance: the first Glyph process listens on a local port; a second
//! launch (e.g. a click in the Windows taskbar jump list, which starts
//! `glyph --dock-action N`) hands its request to the running app and exits.
//!
//! Messages are single lines: `activate`, `dock <index>`, `jam <code>`,
//! `open <link>`, `plugin <id>`, `shared <text>` (a music link sent to
//! Glyph) or `files <path>␟<path>` (audio files opened with Glyph).

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
    glyph_app::links::request_for_url(url)
}

/// A link the system opened Glyph with while it runs (macOS hands URLs to
/// the running app, Android to the activity); the app picks it up like a
/// request of a second launch.
pub fn open_url(url: &str) {
    // Android share sheet: MainActivity hands shared text over this way.
    if let Some(text) = url.strip_prefix(SHARED_PREFIX) {
        open_shared(text);
        return;
    }
    if let Some(req) = request_for_url(url) {
        if let Ok(mut v) = OPENED.lock() {
            v.push(req);
        }
    }
}

/// Marks text shared to Glyph (`MainActivity.takeLink`), not a link.
pub const SHARED_PREFIX: &str = glyph_app::links::SHARED_PREFIX;

/// Text another app shared to Glyph: its links (a song, a playlist), or the
/// text itself when it holds none (a song name, searched for). The app
/// handles it like text pasted into the search bar.
pub fn open_shared(text: &str) {
    let Some(q) = glyph_app::links::shared_query(text) else {
        return;
    };
    if let Ok(mut v) = OPENED.lock() {
        v.push(format!("shared {q}"));
    }
}

/// A request from inside this process (the tray menu), handled by the
/// app's poll like one of a second launch. Never reachable from other
/// processes: the listener only forwards its own short list.
pub fn post(request: &str) {
    if let Ok(mut v) = OPENED.lock() {
        v.push(request.to_string());
    }
}

/// Separates the paths of one `files` request (never part of a path).
const FILE_SEP: char = '\u{1f}';

/// The `files` request for audio files opened with Glyph (`None`: none of
/// them is an audio file).
pub fn files_request(paths: &[String]) -> Option<String> {
    let files: Vec<&str> = paths
        .iter()
        .map(String::as_str)
        .filter(|p| valid_file(p))
        .take(glyph_app::opened::MAX_FILES)
        .collect();
    (!files.is_empty()).then(|| format!("files {}", files.join(&FILE_SEP.to_string())))
}

/// The paths of a `files` request.
pub fn split_files(list: &str) -> Vec<String> {
    list.split(FILE_SEP).filter(|p| !p.is_empty()).map(str::to_string).collect()
}

fn valid_file(path: &str) -> bool {
    let p = std::path::Path::new(path);
    !path.contains(['\n', '\r', FILE_SEP]) && glyph_core::import::local::is_audio(p) && p.is_file()
}

/// A `files` request names only audio files that exist.
fn valid_files(list: &str) -> bool {
    let files = split_files(list);
    !files.is_empty() && files.len() <= glyph_app::opened::MAX_FILES && files.iter().all(|p| valid_file(p))
}

/// A `shared` request holds exactly what [`open_shared`] makes of a text.
fn valid_shared(text: &str) -> bool {
    glyph_app::links::shared_query(text).as_deref() == Some(text)
}

/// The link inside an internet shortcut: Windows `.url`, Linux `.desktop`
/// links (`URL=...`) or macOS `.webloc` (`<string>...</string>`), as made
/// by dragging a link out of the browser.
fn shortcut_link(path: &str) -> Option<String> {
    let p = std::path::Path::new(path);
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(ext.as_str(), "url" | "desktop" | "webloc") {
        return None;
    }
    if std::fs::metadata(p).ok()?.len() > 64 * 1024 {
        return None;
    }
    let text = std::fs::read_to_string(p).ok()?;
    let link = if ext == "webloc" {
        text.split("<string>").nth(1)?.split("</string>").next()?.trim().to_string()
    } else {
        text.lines().find_map(|l| l.trim().strip_prefix("URL="))?.trim().to_string()
    };
    (link.starts_with("https://") || link.starts_with("http://") || link.starts_with("glyph://")).then_some(link)
}

/// A `file://` URI (what Linux launchers hand over) as a path.
fn file_uri_path(arg: &str) -> Option<String> {
    let rest = arg.strip_prefix("file://")?;
    // `file:///home/a/b%20c.mp3` (no host) -> `/home/a/b c.mp3`.
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    if !path.starts_with('/') {
        return None;
    }
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(b) = hex {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    let path = String::from_utf8(out).ok()?;
    // Windows: `file:///C:/x.mp3`.
    Some(if cfg!(windows) { path.trim_start_matches('/').to_string() } else { path })
}

/// What one command line asks for (without the program name).
fn request_for_args(args: &[String]) -> String {
    let args: Vec<String> = args.iter().map(|a| file_uri_path(a).unwrap_or_else(|| a.clone())).collect();
    let links: Vec<String> = args.iter().filter_map(|a| shortcut_link(a)).collect();
    if let Some(req) = args.iter().chain(&links).find_map(|a| request_for_url(a)) {
        return req;
    }
    if let Some(n) = args.iter().position(|a| a == "--dock-action").and_then(|i| args.get(i + 1)) {
        if n.parse::<usize>().is_ok() {
            return format!("dock {n}");
        }
    }
    // A music link: `glyph <link>`, "Send to" on a browser shortcut.
    if let Some(q) = args
        .iter()
        .chain(&links)
        .filter(|a| a.starts_with("https://") || a.starts_with("http://") || a.starts_with("spotify:"))
        .find_map(|a| glyph_app::links::shared_query(a))
    {
        return format!("shared {q}");
    }
    let plain: Vec<String> = args.iter().filter(|a| !a.starts_with('-')).cloned().collect();
    files_request(&plain).unwrap_or_else(|| "activate".into())
}

/// What this launch asks for, from the command line.
pub fn request_from_args() -> String {
    let args: Vec<String> = std::env::args().skip(1).collect();
    request_for_args(&args)
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
                            if line == "activate" || line.starts_with("dock ") || line.strip_prefix("jam ").is_some_and(|c| glyph_protocol::jam::normalize_code(c).as_deref() == Some(c)) || line.strip_prefix("open ").is_some_and(|s| glyph_core::share::link_id(s).is_some()) || line.strip_prefix("plugin ").is_some_and(glyph_core::plugins::valid_id) || line.strip_prefix("shared ").is_some_and(valid_shared) || line.strip_prefix("files ").is_some_and(valid_files) {
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
        assert_eq!(request_for_url("glyph://plugin/sakura").as_deref(), Some("plugin sakura"));
        assert_eq!(request_for_url("glyph://plugin/Sakura/").as_deref(), Some("plugin sakura"));
        assert_eq!(request_for_url("https://glyph.my/plugins/sakura?ref=x").as_deref(), Some("plugin sakura"));
        assert_eq!(request_for_url("glyph://plugin/../x"), None);
        open_url("glyph://jam/K7P2QX");
        assert_eq!(try_recv().as_deref(), Some("jam K7P2QX"));
    }

    #[test]
    fn request_defaults_to_activate() {
        assert_eq!(request_for_args(&[]), "activate");
    }

    #[test]
    fn music_links_and_files_from_the_command_line() {
        let a = |v: &[&str]| request_for_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(a(&["https://open.spotify.com/track/123"]), "shared https://open.spotify.com/track/123");
        assert_eq!(a(&["--dock-action", "2"]), "dock 2");
        let dir = std::env::temp_dir().join(format!("glyph-instance-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let song = dir.join("song.mp3");
        std::fs::write(&song, b"x").unwrap();
        let notes = dir.join("notes.txt");
        std::fs::write(&notes, b"x").unwrap();
        let song_s = song.to_string_lossy().into_owned();
        assert_eq!(a(&[&song_s, &notes.to_string_lossy()]), format!("files {song_s}"));
        assert!(valid_files(&format!("{song_s}{FILE_SEP}{song_s}")));
        assert!(!valid_files(&notes.to_string_lossy()));
        let lnk = dir.join("Song.url");
        std::fs::write(&lnk, "[InternetShortcut]\r\nURL=https://open.spotify.com/track/9\r\n").unwrap();
        assert_eq!(a(&[&lnk.to_string_lossy()]), "shared https://open.spotify.com/track/9");
        assert!(valid_shared("https://open.spotify.com/track/9"));
        assert!(!valid_shared("  two  words "));
        let uri = file_uri_path("file:///home/a/b%20c.mp3");
        assert_eq!(uri.as_deref(), Some(if cfg!(windows) { "home/a/b c.mp3" } else { "/home/a/b c.mp3" }));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
