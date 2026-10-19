# Glyph

See [CHANGELOG.md](CHANGELOG.md) for the version history and [Releases](https://github.com/glyphmusic/desktop/releases) for downloads.

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

New releases include one `SHA256SUMS.txt` listing every asset. Earlier releases keep individual `.sha256` files. App versions before 0.2.2 need one manual upgrade to read the new manifest.

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

### Fast Windows development

Run `./scripts/run-dev.ps1` to build and open the latest dev app. For an immediate
restart of the already built version use `./scripts/run-dev.ps1 -NoBuild`.
`./scripts/dev-watch.ps1` rebuilds after edits and restarts only on success.
All three share the ordinary Cargo `target` cache and overwrite one
`glyph-run.exe`. Keep the incremental/dependency cache: `cargo clean` makes the
next start slower. Dependencies stay optimized and Windows links with rust-lld.

For quick compiler feedback without linking or launching, use `./scripts/run-dev.ps1 -Check` (or `cargo check -p glyph` on any platform). Add `-Timings` to collect Cargo's HTML compilation report in `target/cargo-timings/`. `-NoBuild` restarts the existing debug binary immediately. Dev scripts share one target directory and use half the logical CPUs, capped at four compiler jobs; set `CARGO_BUILD_JOBS` explicitly to override. Keep the dependency cache for fast incremental edits: removing it forces GPUI, audio codecs and all other dependencies to compile again. Workspace crates already inherit level-1 dev optimization; third-party codecs, FFT and GPUI retain level 3. The desktop binary omits debug tables during normal development; use `cargo build -p glyph --config 'profile.dev.package.glyph.debug=1'` when source-level debugging is needed. Release optimization is unchanged.

With Glyph closed, run `./scripts/measure-startup.ps1` for three process-to-window measurements. This opens and closes only its own test instances and reports the median; it does not measure server/feed readiness. Startup logs separately mark platform, font, window, library and audio initialization. Settings load once alongside platform initialization, translations load per language, and library/audio initialize independently. The optional splash preserves its original 2.9-second animation.

The `fast` profile is for local tests with release-level optimization but without LTO (`cargo build --profile fast -p glyph`). Own workspace crates inherit incremental code generation; third-party dependencies use compact non-incremental artifacts. LTO is explicitly `"off"` so local ThinLTO is disabled as well. It uses `target/fast`, separate from `target/debug`: use one profile consistently for an edit cycle instead of alternating and rebuilding both. Normal `dev` uses less optimization for frequently edited code and is preferable for UI iteration.

### Automatic build retention

Dev launch/check and the watcher run `scripts/clean-builds.ps1` automatically. The watcher retries once a minute when idle. Cleanup never invokes Cargo and defers while any compiler is running. It removes the obsolete `target/ui` tree, inactive profiles unused for 24 hours, old duplicate Cargo executables, and all but five timing reports. The selected profile, running apps and current dependency/incremental caches are retained. For direct Cargo commands, the repository agent rules require the same cleanup afterward with `-KeepProfile debug`, `fast` or `release`. `-Plan` previews the candidates and `-WhatIf` exercises the deletion plan without removing anything. Paths outside this checkout's target directory and linked directories are rejected.

### Download concurrency and source enforcement

`config.toml` controls offline download admission across all lists:

```toml
[audio]
download_parallelism = 5 # maximum, clamped to 1..10
download_adaptive = true # slow: 1; unknown/standard: up to 3; fast: configured maximum
```

There are no UI controls for these two limits. Data saver caps admission at one.
Network changes affect new transfers; running downloads finish normally.

In Playback > Sources, **Use YouTube only** requires the selected Local or Server
method for new lookups. The server receives `youtube_only = true`, searches a
bounded set of variants with extra time, and does not fall back to previews or
other providers when no match is found. The corresponding TOML key is
`providers.youtube_only`; `providers.youtube_mode` selects `"local"` or `"server"`.
