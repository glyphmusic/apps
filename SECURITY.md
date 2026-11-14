# Security policy

## Supported versions

Security fixes ship in new releases on `main`. Only the latest release is
supported; the in-app updater installs releases signed with the Glyph release
key (`release-key.pub`, `SHA256SUMS.txt.minisig`).

| Version | Supported |
| --- | --- |
| Latest release (`0.11.x`) / `main` | Yes |
| Older releases | No — update to the latest release |

## Reporting a vulnerability

Report security vulnerabilities privately through
**[GitHub Security Advisories](https://github.com/glyphmusic/apps/security/advisories/new)**
(also listed in [glyph.my/.well-known/security.txt](https://glyph.my/.well-known/security.txt)).

Do **not** open a public issue or pull request for a security problem.

Please include:

- the Glyph version or commit hash and operating system,
- steps or a minimal proof of concept (manifest, share code, response or file) to reproduce the problem,
- what an attacker can read, change or execute.

## Scope

### In scope for this repository

- Sandbox, permission or host-allowlist escapes in the WebAssembly plugin runtime (`crates/glyph-core`, `net.fetch` private-network/host checks, secret settings isolation)
- Signature or checksum verification bypasses in the in-app updater (`release-key.pub`, `SHA256SUMS.txt.minisig`)
- End-to-end encryption or key-handling flaws in account sync
- Path traversal, arbitrary file access or code execution via plugin manifests, themes, share codes (`glyph:playlist:…`), `glyph://` links, the local MCP server or remote provider/server responses

### Other repositories and channels

- **Server or `glyph.my` / `api.glyph.my` / `docs.glyph.my`:** report on [glyphmusic/server](https://github.com/glyphmusic/server/security/advisories/new).
- **YouTube companion or `r-dlp`:** report on [glyphmusic/companion](https://github.com/glyphmusic/companion/security/advisories/new) or [glyphmusic/r-dlp](https://github.com/glyphmusic/r-dlp/security/advisories/new).
- **A Listed plugin that breaks catalog rules or is harmful:** open an issue on [glyphmusic/plugins](https://github.com/glyphmusic/plugins/issues) so it can be removed from `index.json`.
- **A third-party plugin doing what its declared permissions allow** (for example contacting a host named in its install warning): see [Trust and safety](https://docs.glyph.my/plugins/security). Plugins are made by their authors and installed at the user's own risk.
