# Web player

Glyph's Svelte UI (`glyph-ui/`) also runs as a web player at `/web` on the
Glyph website (`glyph.my/web`). Desktop and phone apps still bundle the UI
locally and do all streaming and library work on the device.

## Backends

`glyph-ui/src/lib/api/backend.ts` picks one of three backends at start-up:

| Kind | File | When it runs |
| --- | --- | --- |
| `tauri` | `src/lib/api/tauri.ts` | Inside a Tauri webview (`__TAURI_INTERNALS__` present) |
| `web` | `src/lib/api/web.ts` | Built with `VITE_GLYPH_WEB=1` (`npm run build:web`) or served under `/web` |
| `demo` | `src/lib/api/demo.ts` | Plain `npm run dev` in a browser (sample data for UI work) |

The `demo` chunk is only imported by the demo path, so `build:web` leaves the
sample catalogue out of `dist-web/`.

## How the web player works

- **Catalog & metadata (`src/lib/api/web-server.ts`):** calls `glyph-server`'s
  public `/v1/*` routes (`GET /v1/search` SSE, `GET /v1/resolve?direct=1`,
  `GET /v1/lyrics`, `GET /v1/similar`, `GET /v1/chart`, `GET /v1/health`).
  Override the server origin with `VITE_GLYPH_SERVER=https://api.glyph.my` if
  the UI is hosted separately from the API.
- **Audio playback (`src/lib/api/web-player.ts`):**
  - Direct streams (Audius, Jamendo, Internet Archive, Radio Browser) and
    30-second previews (iTunes, Deezer) play straight from the provider in an
    `<audio>` element.
  - YouTube tracks play through the official YouTube IFrame API
    (`youtube-nocookie.com`). A compact video frame appears in the bottom-right
    corner while a YouTube track is active, as required by the YouTube ToS.
  - **No audio goes through `glyph-server`.** The server only returns metadata
    and provider URLs (`?direct=1`).
- **Browser storage (`src/lib/api/web-store.ts`):** settings, liked songs,
  playlists, followed artists, saved albums, queue, listening statistics, and
  recent items live in `localStorage` under `glyph.web.*`.
- **Accounts, E2E sync & profiles (`src/lib/api/web-account.ts`, `web-e2e.ts`):**
  users can sign in on `/web` (browser passkey/password flow or in-app handle +
  password), manage their account and devices, edit their public profile, and
  sync their library and listening statistics end-to-end encrypted (`XChaCha20-Poly1305`
  + `X25519` + `Argon2id`) with desktop and phone apps.
- **Listening statistics (`src/lib/api/web-stats.ts`):** when enabled in
  *Settings › Playback › Content*, listening time, plays, and skips are recorded
  locally and synced across devices when signed in.
- **MCP in server-relay mode (`src/lib/api/web-mcp.ts`):** when signed in and
  enabled in *Settings › Integrations*, the web player long-polls
  `GET /v1/mcp/pull`, shows OAuth approval prompts, executes allowed MCP tools,
  and posts results to `POST /v1/mcp/result`.
- **URLs (`src/lib/routes.svelte.ts`):** pages have shareable paths under
  `/web` (`/web`, `/web/search?q=…`, `/web/library`, `/web/liked`,
  `/web/recent`, `/web/stats`, `/web/playlist/<id>`, `/web/artist/<name>`,
  `/web/album/<artist>/<title>`, `/web/profile/<handle>`, `/web/settings`),
  and `glyph-server` serves `/web/index.html` as an SPA fallback for unknown
  subpaths.

## Build and serve

Build the web bundle in `glyph-apps/glyph-ui`:

```bash
npm --prefix glyph-ui run build:web
```

This writes static files with `base: "/web/"` to `glyph-ui/dist-web/`.

To copy the built files into `glyph-server/glyph-web/` (used by the Cloudflare
Worker asset binding and optional native server `--web-dir` / `GLYPH_WEB_DIR`):

```bash
node ../glyph-server/scripts/snapshot-web.mjs --build
```

## App-only features

When `backend().kind === "web"` (`onWeb()` in `src/lib/web.ts`), features that
need native code or local hardware/network access are hidden or show an
*"Available in the app"* card (`src/lib/settings/AppOnly.svelte`) linking to
`https://glyph.my/download`:

- Full SoundCloud and Deezer streams (Deezer plays 30s previews; SoundCloud
  resolves to another source or YouTube embed)
- Offline downloads and local music folders
- `yt-dlp` / `glyph-r-dlp` extraction
- Equalizer, loudness normalization, crossfade, gapless DSP, and output device
  selection
- Chromecast, AirPlay, DLNA / UPnP, and MPD
- Native plugins, local HTTP MCP server (`127.0.0.1`), Discord Rich Presence,
  system tray, autostart, and the desktop mini-player window
