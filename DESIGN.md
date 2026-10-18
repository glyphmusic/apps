# Glyph design

Rules for every screen of the desktop app. Reference look: Liner
(screenshots in `../docs/refs/`). When in doubt, compare against them.

## The app is alive

Glyph should feel animated, never static. Nothing appears, disappears or
moves in a single frame.

- **Things move, they don't jump.** Panels slide in and out (queue, player
  bar), groups fold open and shut (settings sub-tabs), lists make room when
  something is dragged over them, and items glide to their new place when the
  order changes.
- **Every press answers.** Buttons give feedback on click: transport
  controls shrink a little and spring back, toggles pop and fill.
- **Text arrives.** Titles and labels decode in from ASCII noise; covers
  rise out of an ASCII shimmer while they load.
- **Short and eased.** 150 to 350 ms, ease-out. Motion supports the action,
  it never makes the user wait.
- **Calm when idle.** Only the background glow and small accents keep moving
  on their own; nothing blinks for attention.
- Respect reduced motion where the platform asks for it.

Implementation helpers: `Slide` (app.rs) for in/out panels,
`widgets::press` / `press_k` for button pops, `reveal_k` + `decode` for text,
and register any running animation in the app's frame loop
(`app.rs`, "keep rendering while ...") so frames keep coming.

## Look

- Flat UI. Gradients and glow only in the page background; panels, text and
  buttons are flat: no shadows, no glow, no coloured borders.
- ASCII is an accent only: the background raster, `[ BADGES ]`, skeleton
  shimmer, the optional ASCII cover (the real cover drawn in characters).
  Everything else is a normal polished desktop UI: Lucide icons, Inter,
  small sizes (about 13.5 px body).
- One accent colour, the pink. Text stays readable on the glow.
- Toggleable things show their state in the icon itself: filled when on.
- Centre things properly; keep spacing tight and consistent.
- Settings are split into categories (modal with a left nav), never one long
  page.

## Performance

- Build only what is on screen. Long lists use `widgets::virtual_rows`
  (fixed row height, off-screen rows are empty space).
- Scrolling must stay smooth with hundreds of rows.

## Pattern: drag to reorder

Lists the user can order (queue, own playlists, provider priority) are
reordered by dragging the row itself, never with up/down arrows.

- The real row is picked up and follows the cursor (a drag view that looks
  like the row: same height, cover, title; flat lighter surface, no shadow).
- Its place in the list stays empty (the row is hidden, not removed, so the
  list keeps its height).
- The rows between the old and the new place glide one row aside to open a
  gap where it will land. They ease there (about 60 ms time constant), they
  never jump.
- Dropping moves it there; letting go elsewhere puts it back.

Code: `widgets::drag_shift(key, i, Some((from, slot)), pitch)` gives each
row its eased offset; the list's `on_drag_move` turns the cursor's y into
`slot = floor((y - list_top) / pitch)`, `on_drop` commits the move. The
queue (`views::queue_panel`) and playlists (`lists::list_view`, `SongDrag`)
are the reference implementations. Register the animation in the app's
keep-rendering check (`drag_shift_running`).
