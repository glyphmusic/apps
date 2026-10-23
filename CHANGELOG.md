# Changelog

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
