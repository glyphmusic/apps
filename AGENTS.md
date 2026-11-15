# Build retention

- Every release must update `CHANGELOG.md` with its version, date and user-visible changes. Keep `RELEASE_NOTES.md` consistent with that version's changelog entry.

- Build caches of every profile (debug, fast, release, Android) are kept, so unchanged crates are never compiled again. Only stale outputs are deleted to save disk space: old executables and artifacts nobody used for a while.
- After every local Cargo build/test (including `fast`), run `./scripts/clean-builds.ps1 -KeepProfile debug` (use `fast` or `release` for that profile). Never start a build just to clean outputs.
- Dev launch/watch scripts invoke cleanup automatically. The watcher retries deferred cleanup while idle.
- Remove abandoned `target/ui`; never delete a whole profile. Retain the newest executable per Cargo hash family, running executables and five timing reports. Once a day, cargo-sweep removes artifacts of uninstalled Rust toolchains, artifacts unused for 14 days (`-UnusedDays`) and the least recently used ones while `target` is above 50 GB (`-MaxSizeGB`).
- Preserve the incremental/dependency caches of all profiles; deleting them forces expensive full recompilation. Compiled dependencies are also shared across repos and profiles through sccache (`~/.cargo/config.toml`). Never delete source, downloads, app data, or published files outside this checkout's `target`.
- Skip cleanup while Cargo/Rustc is running. Do not stop another build or app to clean it. Refuse paths outside `target` and directory links.
- A cleanup failure must be reported, never silently presented as successful. Do not bypass tool/policy rejections with a different deletion mechanism.

# Pull requests

Before opening a pull request, read https://docs.glyph.my/contributing/pull-requests
(Markdown: https://docs.glyph.my/contributing/pull-requests.md) and follow it:
one issue per pull request, the description template in
`.github/pull_request_template.md`, a `## Unreleased` changelog entry, the
wording check, and a last line `Agent: <model slug> via <tool>`.
