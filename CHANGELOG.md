# Changelog

## 0.13.1 - 2026-11-22

### Added

- **Music videos in Now Playing**: switch Now Playing to **Video** (or pick **Watch video** from a song's menu) to watch the music video across desktop, mobile and web. Filter between Official, Live and Lyric videos, search or switch videos, pick video quality (`Auto` or a specific resolution) and go fullscreen while song audio pauses automatically.
- **Lyrics under cover toggle**: show or hide the live lyric line below the album cover in Now Playing (`Lyrics under cover`).

## 0.13.0 - 2026-11-21

### Added

- **"Not the right one?" for songs**: if a song played or downloaded the wrong version, open its menu, listen to a 30-second preview of other versions, refine the search and pick the right one. Your choice is saved for you and downloaded right away; the match only changes for everyone once enough listeners confirm it.
- **Install Glyph as an app (web player)**: the web player offers an install prompt with step-by-step help for iPhone/iPad (Add to Home Screen), Safari on Mac (Add to Dock) and other browsers, plus new app icons.
- **Active Jams in the Server dashboard**: admins see running Jam rooms with listeners, queue length and how long they have been running.
- **Draft playlists and shareable settings**: new playlists can be named on phones before they are created, and settings can be shared between devices.

### Changed

- **Polish across phone and desktop**: refreshed bottom sheets, share sheet, now-playing sheet, titlebar, playing-bars animation and settings pages (design guide updated).

## 0.12.16 - 2026-11-21

### Added

- **Cancel in-progress downloads**: cancel a running song download or batch playlist download directly from the context menu, download badge or multi-select bar (translated across all locales).

### Changed

- **Fewer Android permissions**: removed unused `ACCESS_WIFI_STATE`, `CHANGE_WIFI_STATE` and `RECEIVE_BOOT_COMPLETED` permissions from the Android app to reduce Google Play Protect false positives while keeping background notification checks working.

### Fixed

- **Batch transfers & list polish**: improved multi-playlist transfer pacing, Deezer/Spotify/keyless import and cover lookups, and list/context-menu interactions across desktop, mobile and web.

## 0.12.15 - 2026-11-21

### Added

- **Quick transfer & batch playlist migration**: paste links/copied songs or drag and drop playlist files (CSV, JSON, M3U, XSPF, TXT) directly into Transfer, filter lists, watch per-list progress with a background banner, retry failed lists, and save large lists immediately with automatic matching on playback.
- **Quick jump, library filter/sort & queue to playlist**: search now offers Quick jump to pages and lists; Library adds filtering and sorting (Pinned first, A–Z, Most songs); multi-select lists get a quick pick bar (queue, add to playlist, download); and the queue can be saved as a new playlist.

### Changed

- **Faster network requests & cleaner scrolling**: enabled HTTP/2 for app HTTP requests, added custom overlay scrollbars, and refactored plugin and state management with Vitest UI tests.

### Fixed

- **Clearing Liked Songs sync**: clearing all Liked Songs now sends a single `liked_clear` sync signal so the clear syncs cleanly across devices.

## 0.12.14 - 2026-11-20

### Added

- **More tools in the Server dashboard**: view and lift active API blocks (abuse blocks, rate limits, YouTube daily limits, login lockouts) with remaining time, sign out individual devices, see each device's app version and device-ban status, and reset an account's profile picture or banner.

### Changed

- **Smoother loading transitions**: pages across desktop, mobile and web (Home, Search, Artist, Releases, Similar, Stats, Profile, Lists, Server and Queue) now crossfade smoothly from loading skeletons to loaded content without blank flashes or layout jumps (instant in performance mode / reduced motion).

## 0.12.13 - 2026-11-20

### Added

- **Server dashboard in the app**: server owners and accounts given the admin role see a new **Server** page in the sidebar (and mobile menu) showing accounts, storage per person, 30-day charts, device limits, sign-out and bans, server limits, cache stats and pinned app versions.
- **Go back from Latest to Stable**: switching the update channel back to Stable while running a newer Latest build now offers the newest stable version instead of saying Glyph is up to date.
- **Server-pinned update channels**: when a server owner pins a specific version for Stable or Latest, apps connected to that server follow that choice automatically.

### Fixed

- **Cleaner sound when sample rates differ**: natively decoded songs (such as 44.1 kHz played on a 48 kHz output) now use windowed-sinc resampling instead of linear interpolation, keeping highs clear without dullness or aliasing.
- **Equalizer and bass boost headroom**: the equalizer now measures the real peak of the full filter curve so neighbouring boosted bands and bass boosts stay punchy without pumping or clipping the limiter.

## 0.12.12 - 2026-11-20

### Added

- **Notifications on phones (Android)**: Glyph tells you when a new version is ready and when an artist you follow puts out something new (several at once become one notification). Tap one to open the update page or the album. Glyph checks about twice a day, also while it is closed. Turn each kind off in the new Settings > Notifications (or in Android's settings).
- **"Update Glyph" when the server needs a newer app**: when a server change no longer works with an older version, that version shows "Update Glyph" over the whole app, with "Update now" (or the download page) instead of half-working screens.
- **Shared links play in the browser**: opening a glyph.my link (playlist, album, song) in a browser now opens the web player with the list ready to play, save to your library or open in the app.
- **Advanced settings toggle**: power-user settings start hidden behind an "Advanced" switch in Settings so new users are not overwhelmed.
- **AI assistants can manage plugins (MCP)**: new `list_plugins`, `browse_plugins`, `install_plugin`, `uninstall_plugin` and `enable_plugin` tools (off by default under Integrations > AI assistants, verified plugins only, notifies the UI on changes).
- **Account storage bar**: Settings > Account shows how much of the server's account storage limit is used and warns when it is almost or completely full.
- **Playlist picture picker on the web**: picking a playlist cover works in the web player and encodes covers and profile pictures as small WebP images across web and desktop.

### Changed

- **Share instead of "Copy link" on phones**: every "Copy link" in the phone app (artists, Jam invites) now opens the share menu, like songs and playlists already did.
- **Fewer confirmations**: copying a link in the share menu only changes its button to "Copied" (no message on top; Android shows its own). Adding a song to a playlist or taking it out again in the "Save to" menu shows only the tick, no message with "Undo". Also no message any more where the screen already shows the change (Jam started or joined, copied buttons in settings, hidden artists shown again, MCP sessions renamed or removed, equalizer preset saved). Adding shared songs to several places says so once.
- **Album names show at once**: album names Glyph looked up for your songs are remembered on the device, so Liked Songs and playlists show them right away when you scroll, also after a restart or a sync. Lookups for new songs run two at a time and appear on the row at once.

### Fixed

- **Bass boost with "Even out volume"**: with volume levelling on, a bass boost now really makes the bass louder (about +5 dB on most songs for +6 dB). Before, the equalizer turned the whole song down to make room, so the boost was hardly audible. Songs levelling already turned down use that room for the boost instead.
- **Sharing from the web player**: the web player could not make glyph.my links (the server refused them) and always fell back to a search link. It now makes real links when you are signed in.
- **Shared playlist preview**: shows "Shared by @name" instead of an empty artist in front of the song count.

## 0.12.11 - 2026-11-20

### Added

- **Save to a playlist from the heart**: right-click a heart (on phones: press and hold) to pick where the song goes: Liked Songs and every playlist show a round tick, filled when the song is in it. Click to add or take it out; the menu stays open so you can pick several, and "New playlist" makes one with the song.
- **Your own background picture**: Settings > Appearance > Background picture shows a picture of yours behind Glyph instead of the glow, with Darken and Blur. Big pictures are made smaller, copies stay on this device (up to 40), and text switches to dark ink on bright pictures.
- **Plugins can restyle the whole interface**: themes and extensions can bring a "look": a font (also their own), rounder or squarer corners, their own icons, background pictures and a style sheet for every part of Glyph (main window, settings, menus, dialogs, toasts, mini player). A look only uses its own files and loads nothing from the internet. A theme's look shows while the theme is used.
- **Settings > Plugins always looks like Glyph**: every look pauses while it is open, so a look can always be turned off there. The new shortcut Ctrl+Alt+P (Cmd+Option+P on a Mac) opens it from anywhere.
- **Hold a heart on phones**: pressing and holding a heart (and the other buttons with a right-click menu) opens its menu, also in the full-screen player and the mini bar.
- **Share to Glyph without leaving the app (Android)**: sharing a song or playlist link, or a music file, to Glyph now opens the "Add to Glyph" sheet right over the app you are in. It shows at once while the link is read, then offers Liked Songs, your playlists, "Save as playlist" for a shared playlist and Play. Done or closing takes you back where you were; nothing is added in the background without asking.

### Changed

- **Volume levelling like Spotify**: "Even out volume" now gives every song one fixed volume for its whole length, measured the way streaming services do (bass counts less, quiet parts and silence are left out). A scream, a loud chorus or a bass drop no longer turns the rest of the song down. Each song is measured once in the background and remembered.
- **Bass boost keeps the volume**: the equalizer no longer changes how loud a song is levelled. Peaks it pushes over the top are caught for a moment only: a bass peak turns only the bass down, everything else stays as it is.
- **Glyph icon in the status bar (Android)**: while music plays, the status bar and the media notification show the Glyph note instead of a play triangle.

### Fixed

- **Crossfade on phones**: at a crossfade the next song often started over, or nothing played while the time ran on, and after that playback could stop with "audio output is not responding". The next song now fades in and plays on. Pressing play after a long pause no longer trips the same check.
- **Songs no longer stop or skip when the sound output hangs**: if the phone's sound output stops taking audio, Glyph opens it again and goes on where the song was.
- **A song whose stream breaks midway goes on from there**: after a network drop the song continues at the same spot instead of ending early and jumping to the next one. An expired link is fetched again and also continues at the same spot instead of starting the song over.
- **"Choose picture" on phones (Android)**: the button for a playlist picture (and the profile picture and playlist import) opened nothing; it now opens the phone's photo or file picker.

## 0.12.10 - 2026-11-19

### Added

- **Device check on first start**: Glyph now checks the computer or phone once before the background starts and picks what suits it: graphics card or processor for the animations (Settings > Appearance > Animations), and performance mode on slow processors, few cores, little memory or software-only graphics.
- **Performance in the welcome tour**: a new page lets you pick full effects or performance mode; the one that suits the device is marked "Recommended", and it says who draws the background.
- **Swipe in the queue (phones)**: swipe a song in the queue to the right to add it to the queue again, to the left to take it out.
- **Queue mark**: songs you added to the queue yourself show a small queue mark in the accent colour before the artist (phone queue and the queue panel on the computer).
- **"Playing from" opens the list (phones)**: in the full-screen player, tap "Playing from …" at the top to go to that playlist, album, Liked Songs or artist.
- **Scroll effects (phones)**: on playlists and albums the cover shrinks step by step as you scroll down (to half its size, fading in the second half) and grows back when you scroll up; on artist pages the photo zooms in the same way and keeps its soft fade at the bottom. The bar at the top only turns dark once the title reaches it.
- **Scrolling keeps its direction**: after scrolling up or down, a scroll that starts again within a moment keeps going up or down, even when it starts on a sideways row (home, artist pages); the row no longer catches it. After swiping a row sideways, it keeps going sideways the same way. Works with the mouse wheel and on touch screens; after a longer pause the rows react as before.
- **Select songs to remove**: in Liked Songs and your own playlists, the new select button (circle with a check) lets you tap several songs and remove them together. "Select all" picks them all, and Liked Songs gets a "Clear Liked Songs" button (it asks first). An undo button shows right after.
- **Turn off recommendations**: Settings > Playback has a switch to hide the "Recommended" rows on Home and under playlists.
- **Mini player menu**: right-click a mini player to switch between the bar and the card, or close it.
- **Pick the solid colour**: with the "Solid" background, a new row lets you pick its colour: the theme's own, a few calm dark ones, or any colour you like.

### Fixed

- **ASCII placeholders move again**: with ASCII effects on, the characters in loading placeholders change again as a band sweeps over them (like in the old app), instead of a gradient sliding over still characters. In performance mode they stand still; before, performance mode turned them into plain boxes.
- **"Show more" under an artist's bio**: it only shows when the bio is longer than the card; before, it was always there, even for a short bio.
- **Black background on some graphics cards**: some graphics cards started the GPU animations but drew nothing. Glyph now draws a test picture first and checks the first frames of the real background against the processor's maths; when the graphics card fails (wrong or black picture, errors, keeps dropping out), the processor takes over at once, a message says so, and the GPU choice is locked on that device until "Try again" in Settings > Appearance > Animations.
- **Black screen when unliking many songs**: each unlike saved and redrew the whole library, so unliking a lot of songs quickly could leave a black screen. Hearts tapped close together are now sent as one change after a short pause, and removing a selection is a single change.
- **Playlist cover after removing songs**: when a playlist went back to one song, its cover kept showing the old mosaic of several covers. It now shows that one song's cover again.
- **Mini players stay on top**: the mini player could slip behind other windows. It is now put back on top every few seconds, without taking the focus.
- **Crash in search on phones**: searching could crash the page when the results held the same artist or song twice. Repeated entries are now left out (search, Home, artist releases).
- **Smoother scrolling on phones**: song rows no longer make the page wait for the app before each scroll step, the cover shrink and photo zoom are now run by the browser itself in step with your finger, the background glow holds still while you scroll, the full-page edge fade is gone, and more rows are ready around the visible ones.
- **Speed cards in the welcome tour**: the pictures on the "Full effects" and "Performance mode" cards stayed empty on some computers. They are now drawn in a simpler way.
- **Long album titles ran over each other**: on artist pages and "Show all" albums, long titles grew past their card and covered the next one (Android and some Windows setups). They are cut off with "…" again.


### Changed

- **Artist pictures in "Recently played"**: an artist you played from now shows their photo instead of the cover of the song you played (home, the full Recently played list and the recent items in search).
- **Welcome tour on phones**: big buttons across the bottom (Back and Next, "Bring my music" on its own row), a larger Skip button and text, and the tour no longer runs under the status and navigation bars.
- **Tighter artist header on phones**: less space between the name, the fans line, the buttons and the songs.
- **Artist photo up to the top on phones**: on artist pages the photo now runs up under the status bar instead of leaving a dark strip above it. Once you scroll, the bar at the top turns dark and covers the status bar too.
- **No listening time on the Home cards**: the "Welcome back" cards no longer show how long you listened, and artist cards there are round on the left, like an artist photo.
- **No more [ BRACKETS ]**: labels like "[ 0 SONGS · 0 FOLDERS ]" are now plain text (local files, the welcome tour, badges).
- **Update channel "Stable" and "Latest"**: the channel choice in Settings > About now reads Stable / Latest instead of Stable / Pre-release.
- **Add songs tabs on one row**: the tabs of the Add songs panel (Suggested, Liked, Recent, Import) always fit on one row now.
- **Tidier playlist header**: the "0/12 saved offline · 0.0 MB" line only shows once a song of the playlist is saved offline.
- **No strip under the status bar (phones)**: the top bar now covers the status bar too and darkens together with it, and lists and playlists run up under the status bar like artist pages, so the cover is no longer cut off at the top.
- **Errors on phones**: when a page fails, phones show a calm full-screen message with big buttons instead of a small card.
- **Settings tabs on phones**: sub-pages (like "Version & updates" and "Support") are now tabs under the title with an accent line, instead of small chips.
- **No outlines**: buttons and choices no longer have thin outlines (Follow, round buttons, the choice cards in Settings, background shapes, text size previews, check boxes, input fields). They stand out by their fill now; the picked choice is the lighter one.
- **Mini player card controls**: the controls on the cover are now two rows (back, play, next, and below shuffle, volume, repeat, share), so nothing is squeezed. The volume button turns the lower row into a volume bar, which nothing covers any more. Very small cards keep only the top row.

## 0.12.9 - 2026-11-19

### Added

- **Share menu on phones** (app and web): sharing a song, playlist, album or your recap now opens a menu from the bottom instead of only copying. It shows a card with the cover and title, the link, and buttons for Copy link, WhatsApp, Telegram, SMS, e-mail and More (the phone's own share menu). The desktop still copies the link right away.
- **Share songs to Glyph**: sharing a song link to Glyph (Android share menu, or on the computer "Send to" / `glyph <link>`) now opens a small window where you pick where it goes: Liked Songs (ticked at the start), one or more playlists, or a new playlist. A Play button plays it first. Before, the song always went into Liked Songs.
- **Open music files with Glyph**: "Open with -> Glyph" for MP3, FLAC, OGG, M4A, AAC, WAV and MKA files on Windows, macOS, Linux and Android. On the computer the files play at once (several selected files play as one list); "Send to -> Glyph" works too, also for a link dragged out of the browser (a .url file). On the phone the same pick window opens. Songs from such files keep playing from the file after you liked or added them.
- **Copied music links** (computer): copy a Spotify, YouTube or other song or playlist link in another app, switch to Glyph, and a message offers "Add". Can be turned off in Settings > System > "Offer copied music links".

### Fixed

- **Android app stuck: artists, playlists and the start never loaded**: two parts of the app (the media controls, ten times a second, and the jam / output check) could wait for each other forever. Then nothing in the app moved any more: artist pages kept their placeholders, lists did not open, the app sometimes showed "Glyph could not start", and it seemed frozen after coming back from another app. They now always take their turns in the same order.

### Changed

- **Artist pages and lists open faster**: opening an artist or a list sent the whole library (all liked songs and playlists) to the screen again, only to move the sidebar order. Now only the parts that changed are sent (10 KB instead of about 240 KB with 1000 liked songs), and lists that did not change are not redrawn. On the emulator Liked Songs and playlists open after 0.1-0.2 s.
- **Artist pages from the server**: the Glyph server now keeps a whole artist page (popular songs, discography, related artists, fans, photo and banner) for 14 days for everyone, so the app asks once instead of searching and asking four services itself. A new artist shows after about 0.3-0.5 s, one someone else opened before after about 20 ms. Older servers still work: the app looks up what they do not send.
- **More caching**: artist pages and album song lists are kept on the device (artist pages 30 minutes fresh, shown first and refreshed behind for 14 days; albums a week) and also after a restart; offline, the last copy shows instead of an empty page. The app keeps more artist pages (12) and cover lookups (800) in memory before letting go of them.
- **Coming back from another app (Android)**: library changes made while the app was away (songs played, skipped with the media buttons) are sent once on return instead of piling up, and the app only asks for the player and the queue on return instead of the whole app state.

## 0.12.8 - 2026-11-18

### Added

- **Animations: graphics card or processor** (Settings > Appearance > Animations): who draws the background. "Graphics card (GPU)", the default, now computes the whole glow field on the graphics card too (WebGL 2), not only the characters, so the processor stays almost free; a device without WebGL 2 falls back to the processor and the setting says so. "Processor (CPU)" draws everything without the graphics card, for graphics cards that flicker or show a black background. It stays on the device it was set on (`visuals.render_mode`, not synced). Switching takes effect at once.

### Fixed

- **Android app froze after coming back**: while the app was in the background, every library, queue and player update piled up in the paused page and all ran at once on return. Now they wait in the app and only the newest of each is handed over when you come back.
- **Background characters gone after coming back (phones)**: the phone drops the GPU drawing while the app is away; it is now built again when the app returns.

### Changed

- **Songs start faster**: starting a song no longer sends the whole library to the screen before the song is looked up; "recently played" follows a moment later in one update. A song whose source is already being looked up starts as soon as that is done (checked every 25 ms instead of every 100 ms).
- **Search feels quicker**: it starts 300 ms after you stop typing (was 500 ms), and the first hit's source is looked up as soon as it shows, so tapping it plays at once even while other sources still answer.
- **Smoother song lists**: liked and downloaded marks are checked with a quick lookup instead of going through all liked or downloaded songs for every row.
- **Less work while a song plays**: the player state went to the screen ten times a second only because the time moved, and every time the whole player redrew. The screen counts the time on its own, so the state now goes only when something else changes, when the time is off by more than 0.75 s (a seek, buffering) or every 5 s. The time and progress bar now update ten times a second instead of thirty.
- **Lighter background**: the glow field's noise runs without helper functions per cell, and the brightness under text is worked out only when it is checked (about twice a second) instead of for every cell of every frame. It looks the same.
- **Desktop housekeeping**: the 10-per-second task no longer copies the whole settings; while the window is minimized the memory guard checks every 3 s instead of every 0.5 s and sets power saving only when something changed; when memory is high, it empties the working sets every 4 to 10 s instead of every 2 to 5 s and leaves the process that plays the audio alone.
- **Phone layout**: the page runs on under the tab bar and mini player again, so you see it there through a soft fade instead of an empty dark strip.

## 0.12.7 - 2026-11-18

### Fixed

- **Covers stopped loading (phone Home)**: a cover request whose answer never arrived (a lost bridge reply on Android, a stuck download) held one of the five loading slots for good; once all were held, no cover loaded anymore, typically after scrolling down Home. A slot is now freed after 6 s and a request counts as "no cover" after 20 s (asked again later).

### Changed

- **Phone layout**: no dark fill behind the tab bar and mini player any more; the page ends above them with a short fade, so the app background shows there.

## 0.12.6 - 2026-11-18

### Added

- **Performance mode** (Settings > Appearance > Performance): halves the background's frame rate, samples the glow field on a coarser grid, redraws the ASCII characters less often, and turns off ASCII decode effects and the blur behind panels. It stays on the device it was set on (`visuals.performance_mode`, not synced).
- **Settings pictures**: streaming quality is now four picture cards (sound bars and about how much data an hour uses) instead of a dropdown; performance mode has its own picture card.

### Changed

- **Background on the GPU**: the ASCII characters and titlebar dots of the background are drawn by a WebGL shader (`glow-gl.ts`) from the small glow field instead of the CPU filling and uploading a full-window bitmap 6-12 times a second. Without WebGL the CPU path stays.
- **Caching**: album pages, the Home feeds and artist photos are kept across restarts, so they show at once and refresh quietly behind; a failed refresh never empties a page that already shows songs. The web player mirrors a library too big for localStorage into IndexedDB (playlists were empty after a reload until the next sync) and keeps found cover addresses for 7 days.

### Fixed

- **Import order**: sync pushed changed items in key order instead of library order, so liked songs imported on one device appeared shuffled on the others. Pushes and re-encryption pushes now keep the library's order; the web player also pushes liked songs oldest first and keeps a changed playlist (or liked song) in its place instead of moving it.
- **Covers and sidebar icons**: covers asked for while the app was still starting got "no cover" for good, so sidebar entries kept placeholder icons; they are now asked again once the core is up. Newly visible covers load first (newest request first, 5 at a time), playlist mosaics load their first four covers together and cancel when scrolled away, a missing cover is tried once more after 16 s, and sidebar artists show their photo.
- **Smoother lists**: album names and covers found while covers load update the library in one batched event per second instead of one full library event per cover.
- **Android black screen**: after coming back from the background, the app checks that the page still answers; a frozen WebView is recreated and an empty page is reloaded, instead of needing a cache wipe or force close.
- **Playback stopping**: a short network gap while finding the next song no longer skips song after song and stops; the same song is asked for again after 2, 4 and 6 s. Mid-song stream refills wait up to about 11 s (Wi-Fi to mobile data) before giving up, and refused links (4xx) go to re-resolving at once.

## 0.12.5 - 2026-11-16

### Fixed

- **Listening statistics**: a track credited to several artists (`A feat. B`, `A & B`, `A, B`) stored the track's main artist as the display name of every credited artist, so "Top artists" could list the same name twice and the page failed with Svelte's `each_key_duplicate`. Each credited artist now keeps its own spelling, and top artists never repeat a name (existing statistics included).

## 0.12.4 - 2026-11-16

### Added

- **Web Connect (`/web`)**: added browser remote playback support (`web-connect.ts`) over `/v1/devices/playback/*` so web listeners can view their online desktop/mobile devices and control or transfer playback from the Connect panel.
- **Home Quick Picks & Metadata Backfill**: tracks playback source list (`RecentItem::Song.from`), scores ranked quick-pick cards for Home (`home_top`, `ListCard.svelte`), and adds `track_album` / `backfill_track_meta` to populate missing album names and upgrade YouTube thumbnails to album art across library entries.

### Fixed

- **Reconnect & Server URL Migration**: added automatic exponential-backoff reconnection when returning online or recovering from server outages (`controller.rs`), and migrated legacy `server.glyphmusic.workers.dev` URLs to `https://api.glyph.my` (`config.rs`, docs).
- **Desktop & UI**: refined Tauri window capabilities/desktop state handling, player/queue/artist/album/profile views, cover fallback consistency, keyboard shortcuts, and English/German localization strings.

## 0.12.3 - 2026-11-16

### Fixed

- **Artwork & Deduplication**: prefer non-YouTube cover artwork when merging or importing tracks across providers (`prefer_cover`) and automatically upgrade cached 4:3 YouTube thumbnails on disk to square album covers (`cover.rs`).
- **Playback, Preload & Radio**: emit `PreloadFailed` when a preloaded next track fails to open or decode so preload state resets cleanly (`glyph-audio`), clear stale player position/loading flags on track switches (`player_stale`, `restored_pos`), and surface `radio_loading` / offline guards properly during radio continuation.
- **Android Lifecycle & Connectivity**: added `nativeDetach` / `nativeResumed` Activity hooks, bounded event queuing and WebView render-process / main-frame retry recovery (`MainActivity.java`), media service resilience (`PlaybackService.java`), and immediate `set_offline_now` state propagation on platform connectivity changes.
- **Web Player (`/web`) & UI**:
  - Added web PWA manifest (`manifest.webmanifest`) and app icons (`icon-256.png`, `icon-512.png`).
  - Improved web audio background/visibility playback recovery, sync journal canonicalization with `429` retry/cooldown backoff (`web-account.ts`), and `429` backoff during login polling (`account-state.svelte.ts`).
  - Guarded startup against uncaught window errors/rejections (`App.svelte`), added a splash safety timeout (`Splash.svelte`), guarded missing 2D canvas contexts (`glow.ts`), and made `NowPlayingSheet` solid (`#080808`) without translucent blur artifacts.

## 0.12.2 - 2026-11-16

### Fixed

- **Desktop**: made the `mini_show` command asynchronous and thread-safe so opening the desktop mini player no longer deadlocks or freezes the main WebView2 window while audio keeps playing.
- **Mobile & Android**: fixed Android connectivity detection (`NET_CAPABILITY_INTERNET` without requiring strict captive-portal validation) and cleared provider cooldowns/server outage state automatically when network connectivity returns so server search, local search, and playback recover cleanly after network switches.
- **Web player (`/web`)**:
  - Added `/v1/cover` lookup fallback (with deduplicated concurrency-limited LRU caching) so Last.fm (`[LF]`) and synced tracks load cover artwork on Home and lists.
  - Handled `401 Unauthorized` responses on `/v1/account`, `/v1/devices`, and `/v1/sync/*` with a single deduplicated token refresh retry and clean session reset when revoked.
- **UI & Performance**:
  - Virtualized mobile playlist/album/liked/downloads lists (`PhoneListView`) and desktop Recent lists (`ListView`), unloaded offscreen thumbnails (`Cover` and `ListCover`), loaded playlist mosaic covers progressively as rows scroll into view, and incrementally paginated long queues (`BottomSheet`).
  - Removed browser/WebView focus rings and mobile tap highlight boxes on click/tap, made the mobile bottom navigation and mini player bar solid surfaces, and hid visible scrollbars in mobile layout across Home, Search, Library, and the Now Playing sheets.

## 0.12.1 - 2026-11-16

### Fixed

- **Web player (`/web`)**:
  - Moved the hidden YouTube embed host off-screen and clipped (`clip-path: inset(50%)`) so Chromium compositing never paints a dark rectangle over cards or the player bar.
  - Fixed the web mini player closing immediately on open and added Document Picture-in-Picture support (`documentPictureInPicture`) for a chromeless floating mini player (with popup fallback).
  - Preloaded upcoming queue tracks (stream URL resolution cache, cover pre-warming, and secondary `<audio>` pre-buffering for up to 3 upcoming tracks plus hover prefetch).
  - Batched E2E encrypted sync pushes (`<= 250` items / `1.4 MB` per request) so large libraries do not trigger `413 Payload Too Large`.
  - Fixed web login (Argon2 WASM CSP), keybind guard (`keys.ts`), signed-out `/web` hiding Listening Stats and AI Assistants (MCP) settings, artist banner flashing, and mobile search bar focus/tap.
- **Desktop & Android**:
  - Fixed Android startup black screen (`glyph-android` / `MainActivity`) and white transparent desktop mini player window (`#mini`).
  - Added Windows minimize/tray EcoQoS power-saving mode, WebView suspension, and memory trim, plus pausing background ASCII/canvas animation during window resize and UI transitions.

### Changed

- **MCP & Protocol**: added granular MCP settings access modes (`None`, `Read`, `Write`), active MCP session listing/renaming/revocation, `/web#mcp-connect=` pairing prompts, 14-day artist metadata caching, recent searches in empty search view, and borderless segmented pills.

## 0.12.0 - 2026-11-15

### Changed

- **Desktop and Android UI migrated to Tauri 2 + Svelte (`glyph-ui`)**: the desktop app (`glyph-tauri`) and Android WebView shell (`glyph-android`, keeping native `PlaybackService` background playback) now run the shared Svelte interface on top of `glyph-core` and `glyph-audio`, replacing the GPUI UI layer while preserving desktop features (mini player window, tray, media keys, taskbar/Dock integration, Discord Rich Presence, gapless playback, plugins, local HTTP MCP server, and `glyph://` links).
- Linux packages (`.deb` / `.rpm`) now depend on WebKitGTK and GTK 3 (`libwebkit2gtk-4.1-0` / `webkit2gtk4.1`, `libgtk-3-0`, `libayatana-appindicator3-1`) for the Tauri desktop shell.

### Added

- **Web player build (`glyph-ui` web backend)**: adds browser playback mode for `glyph.my/web` using direct provider `<audio>` streams and the official YouTube IFrame embed, browser-local storage (`localStorage`), end-to-end encrypted account sync, listening statistics, and server-relay MCP approval prompts.
- **Public profiles & MCP controls (`glyph-protocol` 0.11.0)**: configurable profile section order (`playlists`, `recent`) and additional MCP playback/queue/device tools (`queue_remove`, `queue_move`, `queue_jump`, `queue_clear`, `start_radio`, `sleep_timer`, `recent_songs`, `list_devices`, `play_on_device`, `select_device`).

## 0.11.0 - 2026-11-14

### Added

- Settings › Account: "Glyph links when sharing" (on by default). Signed in, "Copy link" makes a glyph.my link that plays in the browser, opens in Glyph and works with yt-dlp; off, it copies the song's own link on its platform and lists as a share code.

### Security

- The glib 0.18 and lru 0.12 libraries, both with published security advisories, are gone. Linux no longer needs GTK or WebKit: the official YouTube and SoundCloud player opens in the browser there, and the .deb/.rpm packages stop depending on GTK 3 and WebKitGTK.
- The self-updater uses `zip` 9.5.1 (critical advisory GHSA-m665-v7pm-7grj), and library sync uses `chacha20poly1305` 0.11 (same encryption, existing libraries stay readable).

### Fixed

- macOS: all text shows again. The macOS builds were made without GPUI's text system, so every label, title and value was blank while icons and controls still drew.
- Songs behind YouTube's bot check play again with the built-in r-dlp (r-dlp 0.2.2): its visitor check failed over HTTP/2, so these songs either failed or fell back to the much slower yt-dlp.
- A song clicked right after the start, before Glyph had finished connecting, now plays as soon as it is ready instead of showing "Backend offline" and doing nothing. A search typed in that moment runs again by itself.
- YouTube through the Glyph server: when the server answers without YouTube audio for a song that should come from YouTube (for example with "Use YouTube only"), this computer tries itself with r-dlp before the song counts as not found.

### Changed

- Home appears with the last session's recommendations, popular songs and new releases right away; the fresh rows replace them as they arrive.
- Recommendations load their seed songs side by side, and "New from artists you follow" loads four artists at a time instead of one after another with a pause each.
- Settings › Playback › Sources is split into Search and YouTube, in the order things happen. The switch that turns YouTube and SoundCloud on or off now says so (it was labelled like a player fallback), and the texts no longer call yt-dlp the default.
- Playback errors about YouTube are shown in the app's language instead of an English error message.

## 0.10.1 - 2026-11-02

### Fixed

- Android: the first-start tour's buttons and live illustrations respond to taps again; sideways swipes still turn the page.
- Android: the tour covers the screen fully instead of letting the home screen show through.

## 0.10.0 - 2026-11-02

### Added

- First-start tour: after the splash on a fresh install (desktop and Android), seven short pages introduce search, the library, sound, lyrics, sync and Connect, and colour themes, each with a small live illustration to try out. Arrow keys, Enter and Escape on desktop, swipes on phones; Settings › About › Introduction shows it again.
- A single song link (Spotify, YouTube including Shorts and live links, Deezer) pasted into the search bar adds that song to Liked Songs.
- Android: Glyph appears in the share sheet. Shared song and playlist links, or plain text such as a song name, open like text pasted into the search bar.
- Settings › Playback: "Skip songs that can't play" (on by default) and "Search longer", which tries more videos and other search wordings before a song counts as not found.
- Deleted playlists can be removed from the trash for good.

### Changed

- Local YouTube playback tries the next good matches when the best video will not play and remembers the one that worked.
- The built-in YouTube search retries once after a dropped connection and pauses for one minute only after three failures in a row, instead of ten minutes after the first.

## 0.9.1 - 2026-10-26

### Fixed

- Mobile offline status links directly to Downloads; unavailable songs are dimmed while downloaded, local and fully cached audio remains playable. Search distinguishes unavailable sources from empty results and offers a retry.
- Mobile playlists expose the existing song menu and reuse song data and sort order between frames; covers read only the songs needed for their mosaic.
- Android stops submitting frames when the activity loses focus, keeping the playback task loop responsive for background track preloading and crossfades.
- Touch dragging keeps sliders, equalizer bands, scrollbars and the profile image crop moving beyond the initial gesture threshold.
- Mobile sheets and swipeable rows keep following the finger after moving away from the gesture's starting point.
- Mouse-wheel control works on settings sliders, equalizer bands and mini-player bars, while seek and volume bars keep their configured step sizes.

## 0.9.0 - 2026-10-25

### Added

- **Local files**: Library › Local files plays music from folders on your computer (MP3, FLAC, OGG, M4A, WAV), browsable by folder, artist, album or as one list with its own search. Files play where they are; nothing is copied or uploaded. Tags and embedded covers are read once and only read again when a file changes.
- **Zen mode**: only the cover and the player bar, without sidebar and lyrics. Z, the new button in Now Playing or its view switch turn it on; Esc or the same key ends it.
- **Change every keyboard shortcut**: Settings › About › Shortcuts lists all shortcuts and mouse gestures; click one and press the new key (Esc cancels, Del removes the key, a key another shortcut had moves over). New shortcuts: volume up / down (Ctrl+↑ / ↓), mute (M), like the playing song (Ctrl+Shift+L), shuffle (Ctrl+S), repeat (Ctrl+R), queue (Ctrl+J).
- **Mouse wheel on the player bar**: scrolling over the seek bar jumps 5 s per notch, over the volume 5 %; both steps can be changed or switched off in Settings › Playback › Player.
- **Download button and shortcut in Now Playing**: saves the playing song (Ctrl+D by default); the button can be switched off, the key changed.
- **Choose the download folder** in Settings › Storage. Existing downloads move along; your own files in that folder are never touched or deleted. A finished download's message has a "Show in folder" button (can be switched off).
- **Equalizer: bass and headroom**: an extra bass shelf around 90 Hz and a headroom slider that lowers the level before the equalizer so boosts do not clip.
- **Favourite background visualizer**: Settings › Appearance › Background visualizer keeps one glow shape instead of a new one per song.
- **Text size**: Settings › Appearance › Text size (small to extra large).
- **AI assistants can look things up in the docs**: Glyph's MCP server has two more tools, `docs_search` and `docs_read`, that search and read docs.glyph.my. They are always allowed, whatever the assistant may do otherwise, because they read nothing of yours.

### Changed

- Settings › Sources › "Search" is now "Online search via: This device / Glyph server", so it no longer reads as a search of your own files.
- Lyrics hidden with Ctrl+L stay hidden after a restart (Settings › Playback › Player › Lyrics in Now Playing).

### Fixed

- Changing the download folder preserves the originals until the new index is saved and waits for downloads and freshness checks.
- Closing Settings cancels shortcut recording; local-file playback follows the selected folder and filter, and folder changes during a scan trigger a fresh scan.
- Artist pages said "No songs found" while their albums could be opened and played: songs credited as "Artist - Topic" or together with other artists were not matched, and an artist the search does not know now shows the songs of their latest releases.

## 0.8.0 - 2026-10-23

### Security

- Release files are verified and signed together before publication. Concurrent builds cannot overwrite archives or checksums of a published release.

### Added

- **"?" for songs nobody rated**: YouTube does not say whether a song is explicit. Glyph now looks YouTube results up at Deezer (on the server when search runs there, otherwise from this device); a song found there shows its "E" or nothing, a song found nowhere shows "?" (hover: whether it is explicit cannot be told). "Hide explicit songs" also hides the "?" songs.

- **End-to-end encrypted library**: liked songs, playlists, albums, artists, history, statistics, settings and API keys leave the device encrypted with a key only your devices hold, so the server stores what it cannot read. There is nothing new to remember: signing in with your password opens the library, a new device gets the key through its pairing code or, on the official server, from your other devices. While an account has no password the server keeps the key, so losing a device never loses the library; setting a password replaces that key with one the server never sees. Playlists you show on your profile stay readable, because the server shows them to others. Update every device: older versions do not read encrypted items.
- **Transparency page**: Settings › Server › Transparency lists what the server you use can see (library, API keys, searches and playback, profile, device control, Jam, AI assistants, IP address and ids). Glyph works this out from what it sends itself, not from what the server says. Glyph warns at once when a server does something that only makes sense to read your data: a sign-in salt that would reveal a key, changed encrypted entries, a replaced or removed key, changed pairing messages.
- **What AI assistants may do**: Settings › Integrations › AI assistants now has None, Read or Write for three areas: playback and search, playlists and likes, and listening statistics. Assistants only see the tools you allow, and Glyph refuses every other call, on this computer and through the server. New installs start with Write for playback, Read for playlists and None for statistics, so a song title that tries to instruct an assistant cannot delete or change your playlists.

### Changed

- **Sign-in on other servers stays in the app**: on a server other than the official one, Glyph offers the password form and a pairing code. That server's own sign-in page opens only after a warning, because a modified server could read the password there. The official server keeps its browser page.
- **Pairing codes have four more characters** on servers that support it. The server never sees them; they let the approving device hand the library's key to the new one.
- **Signed updates**: the in-app updater installs a release only when its checksum list carries a valid signature by the Glyph release key for this exact version. Releases without one are shown but not installed; install them by hand if you trust their source.
- Songs that reach you from other people (a Jam host's queue, shared playlist codes, public profiles) lose covers from unknown image hosts, links outside known music sites and malformed ids before Glyph shows or plays them.

### Fixed

- **A modified server could read synced API keys**: it could hand out a sign-in salt that turned the key sent at sign-in into the key that encrypted the API keys. The key that protects your data now comes from a separate derivation that no sign-in salt can match, and Glyph warns when a server tries.
- Plugin setup and permission texts (service keys, lyrics, audio spectrum) are translated into every language Glyph speaks; outside English and German they showed in English.

### Fixed

- Android keeps background playback active while buffering, reports loading separately from playback and avoids redundant media-notification updates.

## 0.7.0 - 2026-10-23

Plugins and extensions, Glyph starting with your computer, real installers on macOS and Linux, and steadier playback on phones.

### Added

- **Plugin extensions** can supply song lyrics, read the audio spectrum, adjust the local equalizer, edit the local queue and show live progress rows. Service keys are set up per plugin and saved in secure storage; plugins that require a key wait for setup. Plugin calls and queues are bounded, direct access to Glyph servers and private networks is blocked, and account, server, provider and update policies remain controlled by Glyph.

- **Plugins**: Settings › Plugins installs colour themes that people publish on GitHub. Listed themes come from the glyphmusic/plugins index, community themes from every repository with the topic glyph-plugin. Plugins are made by other people, not by Glyph, so before the first install of a plugin Glyph asks you to confirm that you install it at your own risk. Glyph loads both straight from GitHub on your device, without the Glyph server. Search the catalog (forgiving, so typos still match), order it by most stars, recently updated or name, and see each plugin's GitHub stars. Installed themes appear in Settings › Appearance. Click a plugin to see its details and colours. Plugins update automatically when the same repository publishes a newer version (Settings › Plugins › Update automatically, on by default). **Install in Glyph** on glyph.my/plugins opens Glyph on that plugin, ready to install (`glyph://plugin/<id>`).
- **Extensions**: plugins can now change far more than colours. An extension can rename any text in Glyph, hide settings rows, add its own page under Settings › Plugins, add sections to Glyph's settings pages and entries to the song menu, and run its own code. The code runs in a sandbox (WebAssembly, with a time and memory budget, stopped after repeated errors) and uses only what its permissions allow: playback and the queue, your library, search, Glyph's settings (never keys, passwords or the server address) and the web hosts it names. Before installing, Glyph lists everything a plugin may do, and asks again when an update wants more. Plugins with code install only from the listed index. Switch any extension off without removing it; anything a plugin adds to Glyph names the plugin.
- **For plugin makers**: drop a folder with a `glyph-plugin.json` (and its `.wasm`) into Glyph's plugins folder and press Settings › Plugins › Plugins folder › Reload to try it before publishing; Reload also restarts its code, and the plugin's page shows what the code logged. Guide and API reference: docs.glyph.my/plugins.
- **Start with your computer** (Windows, macOS, Linux): Settings › System starts Glyph when you sign in. Choose whether it opens its window or waits in the background with an icon in the system tray (macOS: in the Dock). Click the tray icon to open Glyph; its menu also has Play / Pause, Next, Previous and Quit.
- **Glyph installs as a real app on macOS and Linux**: macOS gets a `.dmg` with Glyph.app to drag into Applications, Linux a `.deb` and an `.rpm` that put Glyph into the app launcher and uninstall through the package manager. The portable archives stay. Glyph.app updates itself in place; with the Linux packages, Settings › About links to the new package. If you used an archive before, the launcher entry Glyph made for it is removed, so Glyph is not listed twice.
- **Uninstalling on Windows asks about your data**: delete all app data (downloaded songs, library, playlists, settings, sign-in, caches), only the downloaded songs and caches, or keep everything. The installer also comes in German now.

### Changed

- **Device bans on servers**: the app sends a hardware id, a SHA-256 of the computer's or phone's machine id with a Glyph-only salt, so a server operator can ban a device and the ban survives reinstalling. The machine id itself never leaves the device.
- **New installs play YouTube from this computer with r-dlp**: search still goes through the Glyph server, but YouTube playback starts on "Local" with r-dlp, which needs nothing installed. Existing settings stay as they are.
- **Phone volume keys control the speaker you play on**: while a network device (TV, speaker, Cast, AirPlay, MPD) plays, volume up and down set its volume, also from the lock screen.
- **Pause and volume on network devices react at once**: commands no longer wait behind the device's status checks, and a burst of volume presses goes out as one change.

### Fixed

- **Phone playback in the background no longer stops between songs**: the app stays awake while it finds and loads the next song, and while a network device plays.
- **Scrolling and dragging on phones follow your finger**: a short swipe no longer flings the list at full speed, scrolling stops and coasts like in other Android apps, and taps and swipes no longer stop working after a touch got lost.
- **Playback goes on after the sound output changes**: plugging headphones in or out, or Bluetooth connecting, used to leave the player silently stuck.

## 0.6.1 - 2026-10-22

Devices that follow each other at once, a player that skips songs it cannot play, and a phone app that handles more like one.

### Changed

- **Your devices update instantly**: when you control another signed-in Glyph, a new song, pause, volume or a jump shows up within a moment instead of after several seconds. Glyph also checks in with the server far less often while the live connection is up.
- **A song that cannot play no longer stops the music**: Glyph skips it with a short note and goes on with the next one (after five in a row it stops and says so).
- **Undo after removing**: taking a song out of a playlist, Liked Songs, Recently played or Downloads, and deleting a playlist, shows a toast with an "Undo" button.
- **Sleep timer fades out**: the music gets quieter over the last 30 seconds instead of stopping abruptly; the volume is back when you play again.
- **Radio after a list goes on from it**: when the queue ends with radio on, the new songs are based on the last few songs, join the end of the queue (so "previous" still goes back into the list) and leave out what just played. Every radio also starts faster.
- **Calmer search results**: once results have been on screen for a moment, the top three stay put while more sources answer, so the row you are about to click does not jump away. A song that comes in again with better details moves to its right place, and long result lists rank faster.
- **Shuffle with repeat all mixes every round anew** instead of playing the same shuffled order again, and a round never starts with the song that just ended.
- **Phone lists keep their name in view**: scrolled past the heading, the top bar shows the list's name (and a play button for songs).
- **Long-press menu on phones**: one song header and no Play or Open rows; submenus open as their own sheet with a back arrow.
- **Sheets follow your finger**: pull a sheet down, also straight on from the long press, and let go to close it. The queue and Connect also close from their list.
- Tapping the artist in Now Playing opens their page.
- Searches you leave without picking a result stay in recent searches as text.
- **Text fields edit like everywhere else**: Ctrl+Left/Right (Option on macOS) jump by words, Ctrl+Backspace/Delete remove a word, Shift+Home/End select to the start or end, a double click selects a word and a triple click everything, and Ctrl+Z / Ctrl+Shift+Z (or Ctrl+Y) undo and redo.

### Fixed

- Smart shuffle always puts recently played songs at the end.
- The search history only remembers songs you picked from the search results, not songs started by radio or the play button while the search page was open.
- Sleep timer "end of this song" no longer lets the next song play for a moment before it pauses.
- Android: editing a search field that already has text no longer wipes it.

## 0.6.0 - 2026-10-21

A phone app that feels like one, updates inside Android and smarter search.

### Added

- **Now Playing as a sheet on phones**: it slides up over the page; lyrics open full screen, the queue and Connect as bottom sheets.
- **Swipe gestures on phones**: swipe the mini player, list rows and the side menu.
- **Updates inside the Android app**: Settings > About downloads the new APK, checks it and hands it to Android's installer, no detour over GitHub. The first time, Android asks you to allow Glyph to install apps.
- **Cover and add button in the Android media controls**: the notification and lock screen show the song's cover behind the controls, and a new add button puts the song into Liked Songs (a check once it is there).
- **Smoother switching**: every page change, settings page and sub-tab, search filter, the queue / Connect / "Add songs" panel and its tabs now fade in softly instead of cutting over, and the player's sort and volume menus fade out when they close.

### Changed

- **Better search results**: songs released in your country rank a little higher (nothing is filtered out), artists you searched for count as interests, proper releases rise above uploads without cover or length, and "Artist - Song" uploads no longer push an artist's own releases down.
- Home cards use as few columns as they need, so no card sits alone in a row.
- On large windows the Now Playing cover (and its seek bar) grows up to 360 px.
- Covers that do not load within 4 seconds show the placeholder instead of shimmering forever.
- Phone layout: more even spacing, home cards in two columns, settings rows only wrap when label and control would collide, and Liked Songs explains how to like a song.
- The settings window is taller, so the menu fits with a category open.
- Glyph speaks 8 more languages completely (missing texts filled in).

### Fixed

- The Android app no longer heats up the phone after a few minutes of use: the screen is only redrawn when something changes (at most 60 times a second unless you touch it), the background glow runs at a lower rate, and Now Playing, downloads and the app in the background stop redrawing or polling at full speed.
- Touch scrolling on phones follows only the direction you swipe, so cover rows no longer drift sideways while the page scrolls.
- No stuck highlights after a tap, page titles are no longer cut short, the artist bar stays readable once the photo scrolls away, and empty lists no longer show play buttons.

## 0.5.0 - 2026-10-21

Glyph on your phone, AirPlay, a sleep timer and your library on every device.

### Added

- **Glyph for Android** (preview): the same app on your phone, with a layout made for small screens and playback that keeps going in the background with a media notification. Download `glyph-<version>-android.apk` from the release and install it (it is not in the Play Store yet).
- **AirPlay**: speakers and receivers that speak classic AirPlay (AirPort Express, AV receivers, shairport-sync and many AirPlay speakers) play from the Connect panel. Glyph streams the song itself (lossless, 44.1 kHz) with volume, pause, seek and the song title on the device. Devices that need pairing (newer HomePods, Apple TV) are not supported yet.
- **Your devices in Connect**: other Glyph apps signed in to the same account show up in Connect. Control their playback or move your current queue over to them.
- **Sleep timer**: the timer button in the player bar and Now Playing pauses after 15, 30, 45, 60 or 90 minutes, or at the end of the current song.
- **Sort lists** by title, artist, album or length (sort button on a list page). The choice is remembered per list, and playing follows the order you see.
- **Radio from anything**: "Start radio" on a playlist, album, artist or song mixes songs similar to several songs of the list and spreads out the artists.
- **New from artists you follow** on Home: their releases from the last eight weeks.
- **Your year in Glyph** (with listening statistics on): Statistics > "Your 2026 in Glyph" shows listening time, top artist, favourite hour, day and month and your top songs, which you can save as a playlist or share as a link.

### Changed

- Your account keeps your listening history, recommendations, list choices and statistics. When you sign in, Glyph offers to merge what you did as a guest; signing out switches to the guest library without deleting downloads.
- Playlists can stay on this computer or sync with your account; the playlist page shows which.
- Jam: on glyph.my, changes reach everyone the moment they happen instead of within a few seconds.
- Jam uses less CPU and data, and the jam panel shows the first 20 queued songs ("Show more" for the rest).

## 0.4.1 - 2026-10-20

A small polish release for windows and dialogs.

### Fixed

- Settings and dialogs no longer cover the title bar. You can now move the window and use its buttons while a dialog is open.
- The approval prompt for AI assistants now looks like every other dialog in the app.

## 0.4.0 - 2026-10-20

The big one: bring your music over from other services, a smarter queue, spatial audio and seven new languages.

### Added

- **Transfer music** (Settings > Transfer music, or Library > Transfer music). No login, nothing to set up. From Spotify, copy your liked songs or a playlist in the Spotify app (Ctrl+A, Ctrl+C) and press Paste; from YouTube Music, paste a playlist link or use a Google Takeout file; Deezer and SoundCloud come from a public profile, Last.fm from your user name. It runs in the background, picks up again after a restart, and importing a list twice only adds the new songs.
- **New playlist from a link**: paste a Spotify, Deezer, SoundCloud, YouTube, Apple Music, Bandcamp or Last.fm link instead of typing songs. Playlist files (CSV, Spotify export, M3U, XSPF) work from the same dialog. Pasting a link into search does the same.
- **Up next**: songs you add with "Play next" or "Add to queue" get their own section that always plays first. Click any queued song to jump there.
- **Shuffle styles** (Settings > Playback, or right-click Shuffle): smart shuffle spreads out artists and holds back songs you just heard; fully random is, well, fully random.
- **Spatial audio**: choose Mono, Stereo or Spatial. Spatial keeps 5.1/7.1 surround positions and renders them for headphones.
- **Play on another computer** running Music Player Daemon (MPD), with Glyph as the remote.
- **Listening statistics** (opt-in): time listened, plays, skips, top artists and songs, favourite hours and days. Synced between your devices; turning them off deletes them.
- **Discord Rich Presence** (Settings > Integrations, off by default): show the current song and cover on your Discord profile.
- **AI assistants** can control Glyph (play, queue, search, playlists, likes, lyrics), either locally or through your account with an approval in the app.
- **Lyrics search**: songs found only through their lyrics show up in an "In lyrics" section.
- Right-click menus on the player controls for quick options like mini player style, repeat mode, seeking and output device.
- Artist pages show the full discography, verified badges and an animated equalizer on the playing song.
- Followed artists appear in the sidebar; right-click to hide one.
- Settings > Support > Docs opens the new documentation at docs.glyph.my.
- New languages: Español, Français, Italiano, Polski, Português, Türkçe, Українська.

### Changed

- Lyrics keep the current line centred and glide smoothly from line to line. You can pick another lyrics version and adjust the timing in a new options panel.
- Spotify and YouTube links work without any API keys.
- YouTube search is much faster. You can choose between the built-in r-dlp (default) and yt-dlp.
- Search waits until you stop typing.
- The background animation slows down while Glyph is in the background (you can switch this off), and the window title shows the playing song.

### Fixed

- Server downloads retry short hiccups and fall back to another quality instead of failing.
- "Add to playlist" menus stay on screen and say "No playlists yet" when empty; long playlist names wrap.
- Scrolling on list pages, the playlist info column and the queue panel on the lyrics page.
- Playing an artist only adds the artist to "Recently played", not every song.

## 0.3.0 - 2026-10-19

### Fixed

- Some streamed songs showed a length of 0:00. They now keep their real duration.

### Changed

- Works with servers that only allow signed-in accounts (Glyph Server 0.3.0).

## 0.2.5 - 2026-10-19

More to discover on artist pages.

### Added

- Artists show up in "Recently played" when you play from their page.
- Similar artists at the bottom of every artist page.
- An artist's liked songs and full release list get their own pages.
- The download button tells you how many songs of a list are already saved and how a running download is going.

### Changed

- The radio switch in the queue is greyed out, with an explanation, when the server can't suggest songs.

### Fixed

- Lyrics still load when the server can't fetch them; the app looks them up itself instead of showing an error.

## 0.2.4 - 2026-10-19

Better downloads.

### Added

- **Only YouTube** option in the source settings, for when you want YouTube and nothing else.
- Downloads now share one queue (five at a time by default) that adapts to your connection speed.

### Changed

- YouTube downloads use the same path as playback, so what plays is what you download.

### Fixed

- Two downloads running at once could overwrite each other's temporary files.

## 0.2.3 - 2026-10-19

A smoother, lighter app.

### Added

- Move through song lists with the keyboard.

### Fixed

- Hover, tooltips and sliders no longer get mixed up between the main window and the mini player.
- Editing a playlist on two devices at once no longer loses changes.
- Glow animations keep running while you move the window.
- Russian artist names with separators are split correctly.

### Changed

- Lower memory use: artwork and metadata caches have a size limit, and large libraries no longer slow down long lists.
- Smoother audio with less work on the playback thread.
- The audio stream cache now shows up in the storage settings.

## 0.2.2 - 2026-10-18

Playback starts faster and stops stuttering.

### Added

- **Share links**: when signed in, copy short Glyph links for songs, artists and playlists, with previews. Shared playlists open as a preview you can add to your library.
- Glyph warms up the start of songs you hover, lists you open and the next songs in the queue, so they start instantly (respects data saver).

### Changed

- Downloaded songs play straight away without touching the network. Updated audio is fetched in the background and only swapped in once it's complete and verified.
- Streaming starts playing as soon as the first bytes arrive, also after seeking, and keeps what it has received for replays.
- The next song in the queue preloads earlier, and the seek bar stays where you put it while buffering.
- The playlist editor's Save button only lights up when something actually changed.
- Releases come with a single checksum file. Older versions need one manual update to adopt it.

### Fixed

- Pauses at chunk boundaries while streaming.
- Playback on Windows under load or while minimized.
- Audio devices that accept a stream but never play it no longer load forever; Glyph retries and tells you if the device doesn't respond.
- Playback through proxies that switch between partial and full responses.
- Clicking outside Settings closes it instead of clicking whatever is underneath.
- More space above the queue panel.

## 0.2.1 - 2026-10-18

### Added

- **Choose where YouTube plays from** (Playback > Sources > YouTube playback): your own computer (yt-dlp) or the server. Set per device.
- A proper Windows installer with a Start menu shortcut and uninstaller. Updates replace the existing app instead of installing a second copy.

### Fixed

- Search, playback, profiles and devices failing with "HTTP 400" when signed in.
- Devices and profiles stuck on "loading"; they now show an error and a retry button.
- The app showing up under the wrong name in Windows.

## 0.2.0 - 2026-10-18

The first stable release for every platform.

### Added

- Builds for Windows, macOS (Intel and Apple Silicon) and Linux (x64 and ARM64).

### Changed

- Refreshed interface, playlist controls, profile pictures and mini player.
- Steadier playback: smarter buffering, loudness levelling and recovery from network trouble.
- Better account sync and library activity.

### Fixed

- Missing images, favicon and download page on the website.

## 0.1.0 - 2026-10-17

### Added

- The first public release: search, play, lyrics, library and playlists, with optional account sync.
