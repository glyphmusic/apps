# Changelog

Changes are recorded here for every release, newest first.

## 0.2.1 — 2026-10-18

### Fixed

- Send account authorization only once per request. Duplicate headers caused HTTP 400 responses in search, playback resolution, profiles and device management.
- Show failures and retry actions for device and profile loading instead of leaving loading placeholders visible indefinitely.
- Bound account-refresh waits and local yt-dlp status checks.
- Normalize Windows shortcut paths so the app is registered as **Glyph**.

### Added

- **Playback > Sources > YouTube playback**: choose **Local** yt-dlp or the **Server** companion. The choice is specific to each device. Local mode resolves YouTube before contacting the server; downloaded files always play locally.
- A Windows setup executable with a stable installation folder, Start menu shortcut and uninstaller. New installations update the existing app rather than creating another versioned installation.
- Playback startup diagnostics and a test for starting a downloaded track through the real audio output.

## 0.2.0 — 2026-10-18

### Added and improved

- Updated desktop interface, playlist controls, profile pictures and mini player.
- Adaptive audio playback, improved streaming buffers, loudness processing and playback recovery.
- Improved library activity, account synchronization and private-server access handling.
- Server request admission and abuse limits, persistent audio cache inventory, storage reservations and audio quality variants.
- Stable releases for Windows x64, macOS Intel and Apple Silicon, and Linux x64 and ARM64, with SHA-256 checksums.

### Website fixes after release

- Route `/download`, screenshots, icons, the web manifest and crawler files through the Cloudflare Worker. This fixes missing landing-page images, the unavailable favicon and the download page's HTTP 522 response.

## 0.1.0 — 2026-10-17

- Initial published desktop release: music search, playback, lyrics, library and playlists, optional account synchronization, and portable application archives.
