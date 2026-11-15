# Porting GPUI screens to glyph-ui (Svelte 5)

The desktop UI in `glyph-apps/src/ui` (GPUI) is being moved to Svelte so the
app can run on Tauri. Every screen is rebuilt **from the GPUI code**, value
by value. Read `../DESIGN.md` and `../../docs/RECREATE-DESIGN.md` first.

## Rules

- **Copy the components, not the vibe.** Every size, colour, gap, radius,
  font weight, hover / active / playing / empty / loading state comes from
  the `.rs` file you port. Write the source in a comment at the top
  (`// lists/rows.rs song_row: h 56, ...`).
- Flat UI. Glow only in the background. No shadows, no coloured borders.
  One accent: `var(--accent)`. Mono (`.mono`) only for accents (times,
  badges, `[ 08 SONGS · 39:18 ]`, numbers).
- Motion: 150–350 ms ease-out (`var(--ease)`), nothing jumps. Panels slide,
  toggles pop (use `ToggleButton`), lists animate. Respect reduced motion
  (already global in `app.css`).
- Texts only through `t("key")` / `tf("key", {n})` from `lib/i18n.svelte.ts`
  (same keys as the Rust code, files in `glyph-apps/assets/i18n`). Never
  hard-code English.
- Icons only through `Icon` / `IconButton` / `ToggleButton` (names = files in
  `glyph-apps/assets/icons/ui`). Tooltips: `use:tip={label}` (from
  `lib/ui.svelte.ts`); `IconButton` / `ToggleButton` take `tip.<icon>` by
  themselves.
- Text that sits right on the glow gets `use:ink` (adaptive dark ink).
- Demo data: real artists / songs / album covers live in
  `lib/api/demo-data.ts` so browser previews and product screenshots look realistic.

## Building blocks (use them, do not re-make them)

| Need | Use |
|---|---|
| Tokens | CSS vars in `src/app.css` (`--text --dim --faint --hover --active --card --accent --xs --sm --md --lg ...`) |
| App state + actions | `app` from `lib/state.svelte.ts` (player, queue, library, navigation, menus, toasts, `setConfig`) |
| Backend calls | `backend().call("cmd", {args})` from `lib/api/backend.ts` |
| Types | `lib/api/types.ts` |
| Icons, buttons | `Icon`, `IconButton`, `ToggleButton`, `Button` (`kind`: default / primary / text / pill / round / back / danger), `Switch`, `Chip` |
| Songs | `SongRow` (lists/rows.rs song_row), `Cover`, `ListCover`, `SongTitle` (E badge), `Badges` (`[YT]`), `PlayingBars`, `DownloadBadge`, `PlayGlyph` |
| Bars | `ThinBar` (seek / volume, hover label, wheel) |
| States | `EmptyState` (bars.rs ascii_state), `Skeleton` (ASCII shimmer), `Decode` (text decodes in), `SectionTitle` (heading / section_title) |
| Helpers | `fmtTime`, `fmtMs`, `metaLine` (`lib/fmt.ts`), `press`, `outside`, `tip`, `ink` (`lib/ui.svelte.ts`) |

## Your own files

Each helper owns a set of files (the stubs say who). Fill the stub, add new
components next to it (e.g. `lib/pages/artist/Hero.svelte`) and, if you need
more state, a module of your own (`lib/<area>.svelte.ts`).

**Do not edit shared files** (`app.css`, `state.svelte.ts`, `types.ts`,
`demo.ts`, `demo-data.ts`, `backend.ts`, `components/*`, `shell/*` except
your stubs, `App.svelte`). If you need a change there, describe it in your
final report; the lead adds it.

## Backend commands

Call new commands with `backend().call("name", { ... })` (snake_case names,
camel-free args). For the browser demo, add handlers in a new file
`lib/api/demo/<area>.ts`:

```ts
import type { DemoHandler } from "../demo";
export const handlers: Record<string, DemoHandler> = {
  stats_month: (ctx, a) => ({ ... }),
};
```

List every new command (name, args, result) at the top of that file in a
comment; the Rust/Tauri side implements them from that list.

## Checking

Run `npm run check` in `glyph-apps/glyph-ui` (svelte-check, a few seconds)
before you finish: zero errors. Do not start the dev server or build the
Rust app.
