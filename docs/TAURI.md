# Tauri app

Glyph's desktop and Android apps run the Svelte UI (`glyph-ui/`) on top of
Rust (`glyph-core`, `glyph-audio`). `crates/glyph-tauri` ships the desktop
binary (`glyph`); `crates/glyph-android` + `mobile/android/` wraps the same UI
in an Android WebView shell with native background playback (`PlaybackService`).

## How the parts talk

- **UI → Rust:** the UI calls `backend().call("name", { args })`. In Tauri
  that is `invoke` (`crates/glyph-tauri/src/main.rs`), and on Android it goes
  through `GlyphNative.invoke` (`crates/glyph-app/src/dispatch.rs`). Every
  handler lives in `crates/glyph-app/src/commands.rs`, with arguments in
  snake_case.
- **Rust → UI:** events named `glyph://<name>`: `player`, `queue`, `library`,
  `config`, `backend`, `toast`, `search`. The shapes are in
  `glyph-ui/src/lib/api/types.ts`; `crates/glyph-app/src/dto.rs` builds them.
- **Toasts** carry an i18n `key`, so the UI shows them in the user's language.
- **Covers and artist photos** are files in glyph-core's cache. The webview
  loads them through Tauri's asset protocol, which may only read the cache
  and data folders.
- **A missing command** (a new one not wired yet) does not break the UI: `tauri.ts` logs "not ported yet"
  and returns nothing, like the demo backend in a plain browser.

## Files

| File | What it does |
| --- | --- |
| `crates/glyph-tauri/src/main.rs` | Window, plugins, `#[tauri::command]` wrappers (`generate_handler!`), start-up |
| `crates/glyph-tauri/src/host.rs` | `TauriHost` (`glyph_app::Host` implementation for desktop) |
| `crates/glyph-tauri/src/desktop.rs` | One Glyph at a time, media keys, taskbar buttons, jump list, tray, Discord, window title, mini player window |
| `crates/glyph-app/src/controller.rs` | App state (`queue`, playing songs, gapless, saving, playback controller) |
| `crates/glyph-app/src/commands.rs` | Playing, queue, library, search, pages, settings |
| `crates/glyph-app/src/dispatch.rs` | JSON command dispatcher shared with `glyph-android` |
| `crates/glyph-app/src/account.rs` | Account, sign-in, sync, profile, privacy, Glyph server, other Glyph apps of the account |
| `crates/glyph-app/src/storage.rs` | Downloads, storage, local music, playlist covers and versions |
| `crates/glyph-app/src/discover.rs` | Radio, recommendations, lyrics choice, stats, sharing, sleep timer |
| `crates/glyph-app/src/transfer.rs` | Importing playlists, moving from other apps |
| `crates/glyph-app/src/plugins.rs` | Plugins |
| `crates/glyph-app/src/jam.rs` | Jam, network devices (Cast, AirPlay, UPnP, MPD) |
| `crates/glyph-app/src/system.rs` | Settings codes, EQ codes, autostart, updates, yt-dlp, MCP, diagnostics |
| `crates/glyph-app/src/mcp_tools.rs` | The MCP tools for AI apps |
| `crates/glyph-app/src/dto.rs` | Turns core values into the UI's shapes |
| `crates/glyph-app/src/settings.rs` | Settings that act on the player at once (EQ, loudness, crossfade, ...) |
| `crates/glyph-tauri/tauri.conf.json` | Window (no OS frame, the UI draws its own title bar), CSP, bundle icons |
| `crates/glyph-tauri/capabilities/default.json` | What the UI may do with its windows (main, mini) |

Shared backend helpers (`devices/`, `guard.rs`, `host.rs`, `i18n.rs`,
`links.rs`, `mcp.rs`, `mpd.rs`, `rt.rs`) live in `crates/glyph-app/src/`, while
desktop-only OS integration (`autostart.rs`, `discord.rs`, `instance.rs`,
`integrate.rs`, `jumplist.rs`, `playback_qos.rs`, `thumbbar.rs`,
`uninstall.rs`) lives in `crates/glyph-tauri/src/`.

The mini player is a second window that loads the same UI with `#mini`
(`App.svelte`, `window.ts` `inMiniWindow`).

## Run it

```bash
npm --prefix glyph-ui install
```

```bash
cd crates/glyph-tauri && npx @tauri-apps/cli@2 dev
```

This starts the Svelte dev server (port 1420) and opens the app window.
After a local Cargo build, run `./scripts/clean-builds.ps1 -KeepProfile debug`.

## Status

Every command the UI calls is there (189), and every part of the GPUI app
was compared function by function with the Tauri side and ported
(playback, search, feeds, artist pages, account and sync, jam and devices,
plugins, MCP, import and transfer, downloads, settings, sleep timer, stats,
tray, media keys, taskbar buttons, jump list, Dock menu, Discord, gapless,
mini player, `glyph://` links, uninstall clean-up).
`cargo build -p glyph-tauri` is clean and the app starts. What each screen
does is not tested by hand yet.

To check that nothing is missing, compare `call("...")` in `glyph-ui/src`
(and the demo handlers) with `generate_handler!` in `src/main.rs`.

## Left on purpose

- Profile follow: neither the server nor glyph-core has it (the GPUI app
  had no follow either); `profile_follow` does nothing.
- Error texts that come from glyph-core itself stay English, as in GPUI.

## Not checked yet

- The macOS-only code (Dock menu, `RunEvent::Reopen`) is only compiled on a
  Mac: CI's `check-macos` job does that.
- Releases (`release.yml`, `packaging/`) and CI (`ci.yml`) build `glyph-tauri`
  (binary `glyph`), and `android.yml` packages the Android WebView shell
  with `glyph-ui/dist`.
