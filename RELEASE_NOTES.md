# Glyph 0.2.3

- Add keyboard navigation for song lists and scroll selected rows using their measured position and row height.
- Keep tooltip, hover, slider and scrolling state separate between the main window and mini player.
- Bound artwork and metadata caches; reuse the liked-song index without hashing the entire library for each row.
- Merge concurrent playlist track edits and update liked-track lookups after remote metadata changes.
- Batch listening activity in a lightweight sidecar and serialize library writes, including pending writes at shutdown.
- Reuse audio prefetch workers and decoded sample buffers; initialize buffer cleanup outside the audio callback.
- Keep glow animations running when moving the window and reduce per-frame allocations.
- Include the audio stream cache in storage settings and consistently recognize Russian artist separators.
