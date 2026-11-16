# Glyph 0.12.2 - 2026-11-16

- **Desktop**: fixes a main-window UI freeze/deadlock on Windows when clicking the Mini Player button (`mini_show`).
- **Mobile & Android**: improves connectivity recovery (`NET_CAPABILITY_INTERNET`) and clears stale provider cooldowns on reconnect so Server search, Local search, and playback recover reliably; hides mobile scrollbars across Home/Search/Library/Player, uses solid colors on the mobile bottom bar & mini player, and removes focus/tap blue boxes.
- **Web player (`/web`)**: loads missing Last.fm (`[LF]`) covers via `/v1/cover`, cleanly handles stale session `401` errors on `/v1/account` and `/v1/devices`, and progressively loads/unloads playlist entries, covers, and queue rows on scroll.

See CHANGELOG.md for the complete changes.
