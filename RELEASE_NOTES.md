# Glyph 0.2.1

- Fix duplicate account Authorization headers that caused HTTP 400 errors in search, playback resolution, profiles and device management.
- Choose local yt-dlp or the server companion in Playback > Sources. Local playback bypasses server resolution; downloaded files stay local.
- Bound account refresh and yt-dlp checks, and show retry actions instead of permanent loading placeholders.
- Add a Windows installer with one stable installation folder, a Glyph Start menu entry and an uninstaller. Installing a newer version updates the existing installation and retains library and settings.
- Keep portable archives and SHA-256 checksums for the in-app updater.
