# Glyph 0.2.4

- Download YouTube audio through the configured server/companion path, using the same source resolution as playback.
- Add a shared download queue: maximum five simultaneous downloads by default, configured only through audio.download_parallelism and audio.download_adaptive in TOML.
- Adapt download admission to observed network throughput and stalled transfers, including local YouTube downloads.
- Add Only YouTube in source settings, honoring Local/Server and returning unavailable instead of falling back to another provider or preview.
- Prevent simultaneous downloads from sharing temporary output filenames.
