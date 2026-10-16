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
- Hard rules from the plan apply everywhere: official APIs only, no audio through the server, no yt-dlp / Innertube / scraping, keys only from config/env/secrets.
