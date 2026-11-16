# Glyph 0.12.3 - 2026-11-16

- **Artwork & Playback**: prefers non-YouTube album covers when merging tracks or upgrading cached thumbnails (`cover.rs`), handles `PreloadFailed` cleanly when preloaded next tracks fail to decode, and keeps player position/loading/radio states accurate across switches.
- **Android**: adds `nativeDetach` / `nativeResumed` lifecycle handling, WebView render-process/load recovery (`MainActivity.java`), media service fixes (`PlaybackService.java`), and instant connectivity transitions (`set_offline_now`).
- **Web player (`/web`) & UI**: ships PWA manifest + icons, improves background playback recovery and `429`/sync retry handling, guards startup/splash/canvas edge cases, and gives `NowPlayingSheet` a solid background.

See CHANGELOG.md for the complete changes.
