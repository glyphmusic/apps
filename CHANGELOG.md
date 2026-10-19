# Changelog

Changes are recorded here for every release, newest first.

## 0.2.3 � 2026-10-19

- Add keyboard navigation for song lists and scroll selected rows using their measured position and row height.
- Keep tooltip, hover, slider and scrolling state separate between the main window and mini player.
- Bound artwork and metadata caches; reuse the liked-song index without hashing the entire library for each row.
- Merge concurrent playlist track edits and update liked-track lookups after remote metadata changes.
- Batch listening activity in a lightweight sidecar and serialize library writes, including pending writes at shutdown.
- Reuse audio prefetch workers and decoded sample buffers; initialize buffer cleanup outside the audio callback.
- Keep glow animations running when moving the window and reduce per-frame allocations.
- Include the audio stream cache in storage settings and consistently recognize Russian artist separators.

## 0.2.2 — 2026-10-18

- Speculatively warm the first audio bytes of hovered songs, opened lists and upcoming queue tracks, with bounded background work and data-saver support.
- Play saved downloads locally before any network request; check remote revisions in the background and replace changed audio only after a complete SHA-256-checked download.

- Decode arriving HTTP ranges immediately, including after seeking. Keep received audio ranges in a bounded local disk cache for replays and loops.
- Keep the seek cursor at its requested position while buffering, preload the next queue item early, and resume a prepared crossfade player when a seek skips the fade window.

- Signed-in sharing copies short Glyph links for songs, artists and playlists, with rich previews and desktop deep links. Shared playlists open as a preview with an explicit Add to library action.

- Prefetch HTTP audio blocks before the current buffer runs out, with larger startup and decoded reserves to prevent chunk-boundary pauses.
- Enable Windows multimedia scheduling for the WASAPI callback to improve playback under load and when minimized.

- Local yt-dlp playback starts from HTTP chunks while parallel background ranges fill the cache. Subsequent reads switch to the completed local file; downloaded songs remain immediate local playback.

- Start streamable HTTP MP4 audio from its leading metadata instead of scanning every segment before playback; preserve byte-range seeking after initialization.

- Give the playlist editor Save button the same 8 px corner radius as Follow. Keep it inactive gray until the name, cover or playlist switches differ from the opening state. Changed values enable a white Save button; reverting them disables it again.

- Add 10 px of space above the Queue panel to separate it from the topbar.

- New releases use one `SHA256SUMS.txt` for all assets. The updater selects the checksum by exact filename and also supports older per-asset checksum files. Older app versions need one manual upgrade to adopt this format.

- Clicking outside Settings closes the modal and blocks interaction with controls underneath it.

- Continue playback when a proxy changes from partial ranges to full audio responses. Validate returned byte ranges and keep small full responses seekable for MP3/ID3 probing.
- Server: use the companion to search specifically for matching song audio before falling back to previews; serve consistent byte ranges from both cache hits and fresh media.

- Recover audio outputs that accept a stream but never consume samples by retrying the same device with its negotiated buffer settings. Report a nonresponsive output instead of loading forever.
- Add diagnostics around opening the output and queueing decoded audio.
- Server fixes prepared alongside this build: retry full playback sources before reusing a Deezer preview; avoid redundant YouTube metadata requests and long cooldowns after transient YouTube failures.

Antivirus heuristic reports for the unsigned 0.2.1 installer remain under investigation; no clean VirusTotal result is claimed for this release.

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
