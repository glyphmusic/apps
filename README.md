# Glyph

A desktop music player for Linux, macOS and Windows, written in Rust with [GPUI](https://www.gpui.rs).
Glyph searches many legal music sources at once, plays full tracks where the source allows it
and keeps your library, playlists and downloads in one flat, fast UI with optional ASCII-art covers.

Server: [glyphmusic/server](https://github.com/glyphmusic/server) — the app uses
`https://glyph-server.glyphmusic.workers.dev` by default, or any instance you host yourself.

## Features

- **Search** across Audius, Internet Archive, Deezer, Jamendo, Spotify (metadata) and more, merged into one list.
  `"quoted"` searches match exactly; results are ranked by your taste.
- **Home**: recommended for you, popular right now, recently played.
- **Library**: liked songs, playlists with mosaic covers, share codes (`glyph:playlist:…`) to send a playlist as text, downloads for offline play (only where the source permits it).
- **Artist and album pages** with verified badge, fan count, releases (Single / EP / Album).
- **Playback**: crossfade (0–20 s), volume levels (Loud / Normal / Quiet), loudness normalization, 10-band equalizer with presets, output device selection, radio from a song.
- **Lyrics**: synced, smooth auto-scrolling (LRCLIB).
- **Explicit** "E" badges and an option to hide explicit songs.
- **Accounts** (optional): an `@handle` account syncs library, playlists and settings across up to 5 or 10 devices.
- **Windows** taskbar buttons (like / previous / play / next), in-app updater.
- Languages: English, Deutsch, Русский.

## Download

Prebuilt builds for every platform are on the [Releases](https://github.com/glyphmusic/desktop/releases) page:

| Platform | Asset |
|---|---|
| Windows x64 | `glyph-<version>-x86_64-pc-windows-msvc.zip` |
| macOS Apple Silicon | `glyph-<version>-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `glyph-<version>-x86_64-apple-darwin.tar.gz` |
| Linux x64 | `glyph-<version>-x86_64-unknown-linux-gnu.tar.gz` |
| Linux ARM64 | `glyph-<version>-aarch64-unknown-linux-gnu.tar.gz` |

Each asset has a `.sha256` file next to it.

## Build from source

Requires Rust 1.85 or newer.

```bash
cargo build --release -p glyph
```

Linux needs these packages (Debian/Ubuntu names):

```bash
sudo apt-get install libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev libx11-xcb-dev libxcb1-dev libfontconfig-dev libfreetype-dev libasound2-dev libdbus-1-dev libssl-dev pkg-config libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev
```

## Configuration

Everything works without configuration. To change the server, language or add your own
API keys, copy [config.example.toml](config.example.toml) to:

- Linux: `~/.config/glyph/config.toml`
- macOS: `~/Library/Application Support/glyph/config.toml`
- Windows: `%APPDATA%\glyph\config.toml`

Environment variables (`GLYPH_SERVER_URL`, `GLYPH_YOUTUBE_KEY`, …) override the file and are never written back.
An empty `server_url` switches to direct mode, where the app talks to the providers itself.

## Project layout

| Path | Contents |
|---|---|
| `src/` | The app and its UI (GPUI) |
| `crates/glyph-core` | Providers, search, library, config, sync |
| `crates/glyph-audio` | Playback engine, crossfade, equalizer, loudness |
| `crates/glyph-protocol` | Types shared with the server |
| `assets/` | Fonts, icons, translations |
| `docs/` | Interfaces and account docs |

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
Icons from [Lucide](https://lucide.dev) ([license](assets/icons/ui/LICENSE-lucide.txt)).
