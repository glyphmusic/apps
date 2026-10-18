# Glyph 0.2.2

- Warm audio bytes ahead of clicks and queue transitions. Saved downloads always take priority over server playback.
- Check downloaded audio for changed ETags/dates at most daily, with weekly content-hash checks when revision metadata is unavailable; preserve the old file on failure.

- Start decoding HTTP audio as bytes arrive and reuse received ranges locally for loops and replays (512 MiB cache).
- Hold the requested seek position while buffering, preload the next song early, and fix transitions that stayed paused after seeking past the crossfade window.

- Signed-in sharing copies short Glyph links for songs, artists and playlists, with rich previews and desktop deep links. Shared playlists open as a preview with an explicit Add to library action.

- Prefetch HTTP audio blocks before the current buffer runs out, with larger startup and decoded reserves to prevent chunk-boundary pauses.
- Enable Windows multimedia scheduling for the WASAPI callback to improve playback under load and when minimized.

- Start local YouTube playback from chunks while the complete cache fills in the background.

- Avoid scanning entire streamable MP4 files over the network before the first audio packet.

- Playlist Save button rounded like Follow: gray without changes, white and enabled after editing.

- Give Queue 10 px of space below the topbar.

- Use one `SHA256SUMS.txt` per release; updater reads both the new manifest and legacy sidecars. Earlier app versions need a manual upgrade for this change.

- Close Settings when clicking the backdrop without activating underlying buttons.

- Handle proxies switching from partial to full audio responses; validate ranges and keep small previews seekable for format probing.
- Server changes use companion audio searches for matching songs and preserve byte ranges on cached media.

- Retry a stalled audio output on the same device using its negotiated buffer settings. Show an actionable error if it still does not respond.
- Add output startup diagnostics.
- Server fixes prepared alongside this build retry full sources before a cached Deezer preview and reduce redundant YouTube metadata work and transient-failure cooldowns.
- Antivirus heuristic reports for the unsigned 0.2.1 installer are still being investigated; this build has no verified VirusTotal verdict.
