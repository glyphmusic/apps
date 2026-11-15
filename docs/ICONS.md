# Icons

Glyph draws its own icons. They live in `assets/icons/ui/`, one SVG file per
icon, and the same files are used by the desktop app, the phone app and the
web UI (`glyph-ui`). The look is round and soft, with only a few solid parts.

## The look

1. **24 × 24 grid.** Keep 2 px free at every edge: draw between 2 and 22.
2. **Lines are 2 px**, with round ends and round joins.
3. **Every corner is round.** Big shapes get a radius of 3 to 4, small ones 2.
   Arrow points, triangles and the tip of a pencil are rounded too. A corner
   that is only softened by the round line join is not enough.
4. **Circles and soft curves** beat boxes. A full circle has radius 9.5, a full
   square is 18 × 18 with radius 4. Then both look the same size.
5. **Few outlines.** Small parts are solid: dots, knobs, note heads, arrow
   heads, the pencil's eraser, the fold of a page, ear cups. Big areas stay
   lines, or the icon gets heavy and blurry at 14 px.
6. **One colour.** Use only `currentColor`. No fixed colours, gradients,
   shadows or see-through parts. The app colours the icon.
7. **On / off icons.** Some buttons show their state in the icon: the plain
   file is "off" (only lines), `<name>-fill.svg` is "on" (filled). These are
   `heart`, `pin`, `thumbs-up`, `list-music`, `picture-in-picture-2` and
   `layout-split`. Do not fill parts of their "off" version, or on and off
   look alike.
8. **2 px of air** between separate parts, so they do not melt together when
   the icon is small.
9. **Name it after what it shows**, in kebab-case: `arrow-left`, not
   `back-button`. Buttons can use the same icon for different jobs.

## The template

Every icon file starts like this. Only the shapes inside change.

```svg
<!-- Glyph icon set, see docs/ICONS.md -->
<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
  <circle cx="12" cy="12" r="9.5"/>
</svg>
```

Allowed inside: `<path>`, `<circle>`, `<ellipse>`, `<rect>` (with `rx`).
A solid part gets `fill="currentColor"`. No `<polygon>` or `<polyline>`
(their corners are sharp), no `transform`, no `id`, no `style`.

## How to make a new icon

1. **Start a file.**

   ```bash
   python scripts/icons.py new my-icon
   ```

   This writes `assets/icons/ui/my-icon.svg` from the template.

2. **Draw it.** Two ways:
   - **By hand in a text editor.** Think in grid squares. The path letters you
     need: `M x y` move, `L x y` line, `H x` / `V y` straight line across /
     down, `A r r 0 large sweep x y` arc, `Q cx cy x y` curve, `Z` close.
     Small letters (`l`, `a`, ...) count from the last point instead of
     from the corner of the grid.
   - **In Figma or Inkscape.** Make a 24 × 24 frame, turn on a 1 px grid, draw
     with a 2 px centre stroke, round caps and round joins. Export as SVG,
     then copy only the shapes into the template and remove colours, ids and
     transforms.

3. **Let the helper do the round parts.** It prints finished shapes you
   paste into the file:

   ```bash
   python scripts/icons.py arrow 4,12 20,12
   python scripts/icons.py arc-arrow 30 320
   python scripts/icons.py round-shape 2 7,4.5 19.5,12 7,19.5
   python scripts/icons.py round-line 2.2 6.5,9.5 12,15 17.5,9.5
   python scripts/icons.py star 4
   ```

   `arrow` is a line with a filled, rounded head. `arc-arrow` goes around the
   centre (angles in degrees: 0 right, 90 down; add `ccw` for the other way).
   `round-shape` closes a shape and rounds every corner by the first number,
   `round-line` does the same for an open line (a chevron, a check mark).

4. **Check it.**

   ```bash
   python scripts/icons.py check
   python scripts/icons.py preview
   ```

   `check` finds a wrong header, other line widths, fixed colours and sharp
   polygons. `preview` writes `target/icons.html` with every icon at 48, 20
   and 14 px. Open it and look at the new icon next to its neighbours: is it
   as big, as heavy and as round as they are? Is it still clear at 14 px?

5. **Use it in the app.**
   - Desktop and phone: add the name to the `icons!( ... )` list in
     `src/assets.rs`.
   - Web UI: nothing to do, it picks up every file in the folder.
   - A button with this icon gets a tooltip: add `"tip.my-icon"` to
     `assets/i18n/en.toml` (and the other languages).
   - An on / off icon: draw `my-icon-fill.svg` too and add the name to
     `has_fill` in `src/ui/widgets/buttons.rs` and to `hasFill` in
     `glyph-ui/src/lib/icons.ts`.

## Checklist

- [ ] Drawn inside 2 to 22, lines 2 px.
- [ ] No sharp corner anywhere, arrow points included.
- [ ] Small parts solid, big areas as lines.
- [ ] Only `currentColor`.
- [ ] `python scripts/icons.py check` passes.
- [ ] Looked at in the preview at 14, 20 and 48 px.
- [ ] Added to `src/assets.rs` (and a tooltip if it is a button).
