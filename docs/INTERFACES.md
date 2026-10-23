# Interfaces & ownership

Full product plan: `../docs/PLAN.md` (mirror of the Claude Doc "Prompt: Glyph (Rust + GPUI)", incl. the 16.10.2026 additions).

| Area | Path | Contract |
| --- | --- | --- |
| Wire types | `crates/glyph-protocol` | `src/lib.rs` — shared with `glyph-server`, compiles for wasm32 |
| Core (no GPUI) | `crates/glyph-core` | `src/lib.rs` (`Backend`, `Queue`, `Library`, `Lyrics`, `Config`, `updater`) |
| Playback | `crates/glyph-audio` | `src/lib.rs` (`PlayerController`, `MediaKeys`) |
| Desktop UI | `src/` (crate `glyph`) | consumes the two contracts above |
| Server | `../glyph-server` | serves `glyph-protocol` types on `/v1/*` |

Rules:

- Public items listed in the crate-level `CONTRACT` comments are stable. Add items freely; renaming or changing a signature means updating every user in the same change.
- Every `async fn` on `glyph_core::Backend` is awaitable from GPUI's executor (no tokio context required by the caller).
- Targets: Linux, macOS, Windows. No platform-only crate without a `cfg` fallback.
- Hard rules from the plan apply everywhere: official APIs only, no audio through the server, no Innertube / scraping, keys only from config/env/secrets.
- yt-dlp exception (user decision, 2026-10-17): the desktop app may run the user's own yt-dlp locally for YouTube (`glyph_core::ytdlp`): search via `ytsearchN:`, direct audio URLs played by glyph-audio, downloads through the same path. Validated video ids / queries only, no cookies, no login. Binary: `providers.ytdlp_path` → PATH → app-managed copy (official GitHub release, SHA2-256SUMS checked). Never on the server (datacenter IPs get YouTube's bot check). Without yt-dlp YouTube falls back to the official embed. `Backend::{ytdlp_status, install_ytdlp, refresh_source, prefetch, local_youtube}` expose it to the UI. Speed: the app installs the "onedir" release ZIP (starts in <1 s instead of ~4 s), resolves the next queue songs / a hovered YouTube row ahead in one batch run, and shares video ids (never audio) through the server: `POST /v1/youtube/hint` (`YouTubeHintRequest`) after a sure local match, `ResolveResponse.youtube_hint` for the next client. The server looks the song up in its own catalog (Deezer) and confirms a hint only when the YouTube Data API or its own yt-dlp shows a matching title, the artist's channel and a length within six seconds; install ids never count (see the server's `hints.rs`). googlevideo URLs answer 403 once read at playback pace (~1 min), so a local YouTube song is downloaded completely first (parallel 1 MB ranges with yt-dlp's `http_headers`, one re-resolve on 403) into `cache_dir()/youtube/` (400 MB, oldest out) and played from that file; offline downloads copy it. Defaults: YouTube enabled and first in `providers.priority`; in server mode the local yt-dlp wins over anything YouTube outranks and over previews.
