# Build retention

- Every release must update `CHANGELOG.md` with its version, date and user-visible changes. Keep `RELEASE_NOTES.md` consistent with that version's changelog entry.

- After every local Cargo build/test (including `fast`), run `./scripts/clean-builds.ps1 -KeepProfile debug` (use `fast` or `release` for that profile). Never start a build just to clean outputs.
- Dev launch/watch scripts invoke cleanup automatically. The watcher retries deferred cleanup while idle.
- Remove abandoned `target/ui`; keep the current profile, running executables and other profiles used in the last 24 hours. Retain the newest executable per Cargo hash family and five timing reports.
- Preserve the active incremental/dependency cache; deleting it forces expensive full recompilation. Never delete source, downloads, app data, or published files outside this checkout's `target`.
- Skip cleanup while Cargo/Rustc is running. Do not stop another build or app to clean it. Refuse paths outside `target` and directory links.
- A cleanup failure must be reported, never silently presented as successful. Do not bypass tool/policy rejections with a different deletion mechanism.
