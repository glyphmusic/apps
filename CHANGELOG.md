# Changelog

## 0.4.1 - 2026-10-20

- Settings and dialogs no longer cover the titlebar: the window can be dragged (and the window buttons used) while they are open, without closing them.
- The AI assistant approval looks like the other dialogs.

## 0.4.0 - 2026-10-20

- Transfer music (Settings > Transfer music, or Library > Transfer music), with no login and nothing to set up: Spotify liked songs and playlists come over by copying them in the Spotify app (Ctrl+A, Ctrl+C) and pressing Paste; YouTube Music by playlist link or a Google Takeout file; Deezer and SoundCloud from a public profile; Last.fm from the user name. Runs in the background, continues after a restart, and lists that come over twice only get their new songs. Pasting many links into search opens it too.
- Links without keys: Spotify playlists, albums, artists and songs are read from Spotify's public embed page when neither the server nor the app has a Spotify key (up to 100 songs per list), YouTube playlists from YouTube's web endpoint without an API key.
- New playlist: paste a link instead of typing songs. Spotify playlists, albums, artists and songs, Deezer playlists, albums, artists and profiles, SoundCloud sets, likes and tracks, YouTube playlists, public Apple Music playlists and albums, Bandcamp albums, Last.fm profiles and short links (on.soundcloud.com, deezer.page.link, spotify.link) work; pasting such a link into search does the same. Playlist files (CSV, Spotify export, M3U, XSPF) import from the same dialog.
- Settings > Support > Docs opens the new documentation at docs.glyph.my.
- Add Discord Rich Presence (Settings > Integrations, off by default): the current song, artist, album and cover appear in your Discord profile. A Discord application ID from the Developer Portal is needed unless this build ships one.
- Queue: songs added with Play next / Add to queue form an "Up next" section that always plays first; the rest is labelled "Next from: <list>". Click any queued song to continue from there.
- Shuffle styles (Settings > Playback, or right-click Shuffle): smart shuffle spreads artists and moves recently heard songs back; fully random is uniform.
- Right-click menus on player controls: mini player card/bar, repeat mode, shuffle style, seek 10/30 s and restart on previous/next, output device, channel mode and MPD on volume.
- Spatial audio: choose Mono, Stereo or Spatial; Spatial keeps surround (5.1/7.1) speaker positions and renders them for headphones with an HRTF head model.
- Play on a Music Player Daemon (MPD) on another computer, with Glyph as remote.
- Opt-in listening statistics (time, plays, skips, top artists and songs, hours and weekdays), synced across signed-in devices; turning them off deletes them.
- Lyrics: the current line stays centred and moves smoothly with an eased colour sweep; pick another lyrics version (remembered per song) and adjust timing in a new options panel; redesigned view switch.
- Search lyrics: results found only through their lyrics appear in an "In lyrics" section. Search waits until you stop typing.
- AI assistants (MCP): a local MCP server (127.0.0.1, token) or access through the signed-in server with OAuth sign-in and in-app approval; 27 tools for playback, queue, search, playlists, likes and lyrics.
- YouTube: choose r-dlp (built in, default) or yt-dlp; native InnerTube search and a YouTube ID hint to the server make finding sources much faster.
- Server downloads retry transient failures and fall back to another quality instead of failing.
- Artist pages list the full discography from Deezer; verified badges appear next to artist names; an animated equalizer marks the playing song.
- Sidebar shows followed artists; right-click an entry to hide it. Playing an artist adds only the artist to Recently played.
- Add-to-playlist flyouts stay on screen with a 5 px gap and say "No playlists yet" when empty; playlist names are limited to 100 characters and long titles wrap.
- List pages scroll under the top bar; the info column of a playlist scrolls in small windows; the queue panel has bottom padding on the lyrics page.
- The background animation drops to 10 fps while Glyph is not focused (switchable); the window title follows the playing song while the app is in the background.
- New languages: Español, Français, Italiano, Polski, Português, Türkçe, Українська.

## 0.3.0 - 2026-10-19

- Support streamed fragmented MP4 audio without replacing track metadata with a zero duration.
- Recognize the optional account-only API policy advertised by Glyph Server 0.3.0.


## 0.2.5 - 2026-10-19

- Show artists in "Recently played": playing from an artist's page adds the artist next to the songs, on Home and in the list.
- Show similar artists at the bottom of an artist's page (Last.fm with your own key, Deezer otherwise).
- Grey out the queue's radio switch, with an explanation on hover, when the server has no working Last.fm key.
- Look lyrics up locally when the server cannot serve them (for example when LRCLIB rate-limits it) instead of showing an error.
- Give an artist's liked songs and their full release list their own pages, with a round play button next to the heading.
- Show in the download tooltip how many songs of a list are already saved and how an ongoing download is progressing.

## 0.2.4 - 2026-10-19

- Download YouTube audio through the configured server/companion path, using the same source resolution as playback.
- Add a shared download queue: maximum five simultaneous downloads by default, configured only through audio.download_parallelism and audio.download_adaptive in TOML.
- Adapt download admission to observed network throughput and stalled transfers, including local YouTube downloads.
- Add Only YouTube in source settings, honoring Local/Server and returning unavailable instead of falling back to another provider or preview.
- Prevent simultaneous downloads from sharing temporary output filenames.

Changes are recorded here for every release, newest first.

## 0.2.3 — 2026-10-19

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
