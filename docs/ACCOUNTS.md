# Accounts & sync (design, approved 17.10.2026)

Optional. Without an account Glyph works exactly as today: the library stays in
`library.json` on the device. With an account the same library follows the user
to every signed-in device (desktop now, Android/iOS later). Everything runs on
the existing Cloudflare Worker (`glyph-server.glyphmusic.workers.dev`); no other
service is needed.

## 1. Account model

| Thing | Rule |
| --- | --- |
| Handle | `@name`, 3-20 chars `[a-z0-9_]`, stored lowercase, unique. First come, first served. A short reserved list (`glyph`, `admin`, `support`, ...) is refused. Changeable once per 30 days; a released handle stays blocked for 30 days so nobody can impersonate the old owner. |
| Sign-in | **Handle + password** (in the app and on the login page) or **passkeys** (WebAuthn, ES256/RS256). The device derives `key = Argon2id(password, salt)` (19 MiB, t=2, p=1, 32 bytes, `glyph_protocol::password_kdf`); the server stores the salt and SHA-256 of the key only, so the password never reaches it and the Worker stays within the free-plan CPU limit. Passkeys store only the public key. Either method can be added later from Settings -> Account. |
| Second device | **Pairing code** (device-code flow): the new device shows `K7PX-2QMD`, an already signed-in device approves it under Settings -> Account. Works on Linux without any passkey support. |
| Recovery | One recovery code (`glyph-rc-...`, 128 bit) shown once at sign-up, stored only as a hash. Passkeys from iCloud/Google Password Manager also survive a lost device. |
| Email magic link | Optional, off by default. Only if Cloudflare Email Service (send to any address) is enabled on the account; then the address is stored for sign-in only. Not part of v1. |
| Tokens | Per device: access token (1 h) + refresh token (90 days, rotated on every use; an old one stops working). Both are random 32 bytes; the server stores SHA-256 hashes only. The app keeps the refresh token in the OS keychain (file in the data folder where there is none), the access token only in memory. |

### Sign-in flow (desktop)

```
app                         worker                        browser page (worker-served)
 | POST /v1/account/login/start  ->  login_id, user_code, poll_secret, url
 | opens url in browser ------------------------------------> passkey: sign up (pick @handle) or sign in
 |                                                           (or: approve user_code on another device)
 | POST /v1/account/login/poll (every 2 s, 10 min max)
 |   <- pending | {access, refresh, device_id, account}
```

The browser page is plain HTML+JS served by the Worker at `/account/login/{id}`,
in the app's ASCII style. Polling instead of a localhost redirect keeps it simple
on every OS and behind firewalls.

### Relying-party domain

Decision (17.10.2026): the Worker runs on `api.glyph.my` (a Worker Custom
Domain; `workers.dev` stays reachable) and passkeys are bound to `glyph.my`
(`ACCOUNT_RP_ID`), so any `*.glyph.my` page can use them. The app's default
server is `https://api.glyph.my`; a saved old workers.dev URL maps to it.
Changing the RP ID again invalidates every passkey (pairing code, password and
recovery code still work, so nobody is locked out). Mobile apps also need
`/.well-known/assetlinks.json` and `/.well-known/apple-app-site-association` on
that domain; the Worker can serve both.

## 2. Storage (all Cloudflare)

| Data | Where | Why |
| --- | --- | --- |
| users, handles, passkeys, devices, pending logins | **D1** (`GLYPH_DB`) | relational, unique constraints for handles |
| library items (sync) | **D1** too, table `sync_items` | strongly consistent and atomic per batch; KV is eventually consistent (up to 60 s), which breaks "pull after push" on a second device; a Durable Object would also work but adds a class binding and more code in the Rust Worker for no gain at this size |
| cover cache | **R2** (`GLYPH_COVERS`), optional | see section 4 |
| existing caches | KV (`GLYPH_KV`), unchanged | |

Free tiers are enough for personal use: D1 5 GB / 100k writes a day, R2 10 GB.
A library of 5,000 liked songs is roughly 2-3 MB of rows.

```sql
users(id TEXT PK, handle TEXT UNIQUE, created_at INT, device_limit INT DEFAULT 5,
      rev INT DEFAULT 0, recovery_hash TEXT, handle_changed_at INT)
handle_holds(handle TEXT PK, until INT)                 -- released handles
passkeys(cred_id TEXT PK, user_id TEXT, public_key BLOB, sign_count INT, created_at INT, label TEXT)
devices(id TEXT PK, user_id TEXT, name TEXT, platform TEXT, created_at INT, last_seen INT,
        access_hash TEXT UNIQUE, access_exp INT, refresh_hash TEXT UNIQUE, refresh_exp INT)
logins(id TEXT PK, poll_hash TEXT, user_code TEXT UNIQUE, challenge TEXT,
       user_id TEXT NULL, expires INT)                  -- 10 min, then deleted
sync_items(user_id TEXT, kind TEXT, key TEXT, value TEXT NULL, updated_at INT,
           device_id TEXT, rev INT, deleted INT, PRIMARY KEY(user_id, kind, key))
```

No IP addresses, user agents or listening history are stored.

## 3. Device limit

The limit is a server setting, never a client one (the app is open source, so
the client is untrusted; `PATCH /v1/account` with `device_limit` gets 403).
The operator sets a default (5 until changed, stored in D1 `settings`) and can
give single accounts their own limit (1-100):

    glyph-server remote login https://api.glyph.my
    glyph-server remote accounts default-limit 5
    glyph-server remote accounts limit someone 10      # or `default`
    glyph-server remote accounts show someone

The Worker needs the `ADMIN_PASSWORD_HASH` secret for this (from
`glyph-server hash-password`). A new device is inserted only while the count is
below the limit, in one SQL statement, so parallel sign-ins cannot pass it.
Lowering a limit keeps existing devices signed in. A login
that would exceed it is not finished: the browser page (already authenticated by
the passkey) lists the devices and lets the user remove one first; the pairing
flow does the same on the approving device. Settings -> Account shows all devices
(name, platform, last seen) with a remove button. Removing deletes the row, so
that device's tokens stop working on the next request; the app there falls back
to "signed out" and keeps its local library.

## 4. Covers (optional, R2)

* `GET /v1/covers/{sha256(url)}?u={url}`: the Worker fetches the image once from an
  allow-listed provider host (Deezer, Audius, Jamendo, Archive, SoundCloud, YouTube
  thumbnails, Cover Art Archive), stores it in R2, and serves it with long cache
  headers. Other hosts get 400, so it is no open proxy. Max 2 MB, images only.
* Own playlist pictures (today a local file) are uploaded with
  `PUT /v1/sync/covers/{sha256}` (max 512 KB, JPEG/PNG/WebP) under
  `users/{id}/covers/` so the picture syncs with the playlist.
* Without the R2 binding both routes return 404 and the app loads covers as today.

R2 has to be enabled once in the Cloudflare dashboard (Cloudflare asks for a
payment method even for the free tier). Part of v1; the binding stays commented
in `wrangler.toml` until the bucket exists.

## 5. Sync protocol

Every synced thing is an **item**: `(kind, key) -> value`, last writer wins per
item, deletions are tombstones.

| kind | key | value |
| --- | --- | --- |
| `liked` | track key (provider:id, else isrc) | `Track` |
| `playlist` | playlist id | `{name, tracks, origin, cover?}` (whole playlist; its share code is derived from it as today) |
| `followed` | artist name, lowercase | `{name, track?}` |
| `hidden` | artist name, lowercase | `{name}` |
| `album` | `title\|artist`, lowercase | `{title, artist, tracks}` |
| `search` | song track key or `artist:name` | `SearchItem` |
| `download` | track key | `Track` (only *which* songs; every device downloads the files itself) |
| `setting` | name | small JSON (later: theme, sources order) |

Wire format (`glyph-protocol`, all new fields optional so old clients ignore them):

```json
{ "kind": "liked", "key": "audius:Abc12", "value": { ...Track... },
  "updated_at": 1760700000000, "deleted": false, "rev": 412 }
```

* `POST /v1/sync/push {items}`: for each item the server keeps the one with the
  larger `updated_at` (tie: larger device id). `updated_at` is clamped to
  `server_now + 5 min` against wrong clocks. Accepted items get the next `rev` of
  the user. Reply: `{rev, accepted, stale: [items that lost, with the winning value]}`.
  Max 500 items / 2 MB per call (one item up to 1 MB); the app sends chunks of
  200 items / 512 KB.
* `GET /v1/sync/pull?since={rev}&limit=500`: items with `rev > since`, ordered by
  rev. Reply `{items, rev, more}`.
* Tombstones are purged after 180 days. A device whose `since` is older than the
  purge horizon gets `{reset: true}` and does a full pull, then pushes its local
  items that are newer.
* Client: every local change writes the item into a small outbox
  (`sync_state.json`: outbox, last rev). The app pushes 2 s after the last change,
  pulls on start, on focus and every 5 min. Pull results are merged into the
  `Library` with the same rule, so both sides converge.
* First sign-in on a device that already has a library: local items are pushed
  (merge, nothing is deleted), then a full pull.
* Lists keep their order: liked songs and searches by `updated_at`, playlists carry
  their own track order.
* Not synced: recently played, play/skip counters, lyrics offsets, downloaded
  files themselves, the server URL and other device-local settings.

### Settings and API keys (17.10.2026)

`glyph-core/src/settings_sync.rs`. Settings sync as `setting` items, one per
section (`general` language, `providers` enabled/priority/YouTube budget,
`visuals`, `updates`, `integrations` Discord presence, `audio` without the
output device, `player` shuffle/repeat/queue), last writer wins per section.
Device-bound and never synced:
server address, yt-dlp path, cache size, output device, volume, mini player.
A device pushes settings only after its first full pull (`SyncState::pulled_once`),
so a fresh install does not overwrite the account's settings with defaults.

API keys sync as one `secret` item (`api_keys`), end-to-end encrypted:
XChaCha20-Poly1305 with a key `Argon2id(password, sha256("glyph-settings-v1:" +
account_id)[..16])` (same KDF parameters as sign-in, different salt, so it is
not the sign-in key the server sees). The nonce is derived from key + plaintext,
so unchanged keys give the same item and are not pushed again. Only keys from
the config file sync, never env-provided ones. The key lives in the OS keychain
(`settings-key:{account_id}`, file fallback) and is derived on password sign-up,
sign-in and password change. Devices signed in without the password (browser,
passkey, pairing code) store the encrypted item as locked and show "Unlock" in
Settings -> Account; the typed password is first checked with
`POST /v1/account/password/check` (403 `wrong_password`). A device without the
key never pushes or deletes the item. After a password change elsewhere the old
key no longer opens the item: it is dropped and the device asks again. Accounts
without a password sync settings only.

On the public server the app opens the browser sign-in page by default; a plain
text link switches to the in-app form.

## 6. Endpoints

| Method | Path | Auth |
| --- | --- | --- |
| POST | `/v1/account/login/start` | - |
| POST | `/v1/account/login/poll` | poll secret |
| GET | `/account/login/{id}` (HTML page) | - |
| POST | `/v1/account/passkey/register/options`, `.../register/verify` | login id |
| POST | `/v1/account/passkey/login/options`, `.../login/verify` | login id |
| POST | `/v1/account/login/approve {user_code}` | access token |
| POST | `/v1/account/recover {handle, recovery_code}` | login id |
| POST | `/v1/account/login/devices`, `/v1/account/login/remove_device` | login id + page secret (login page at the device limit) |
| POST | `/v1/account/recovery_code` (new code, old one void) | access token |
| POST | `/v1/account/token/refresh` | refresh token |
| GET | `/v1/account/handle/{h}` | - (availability only) |
| GET / PATCH / DELETE | `/v1/account` (handle, device limit) | access token |
| GET | `/v1/devices` | access token |
| PATCH / DELETE | `/v1/devices/{id}` (rename / remove) | access token |
| POST | `/v1/devices/self/signout` | access token |
| POST / GET | `/v1/sync/push`, `/v1/sync/pull` | access token |
| GET | `/v1/sync/export` (all items as JSON) | access token |
| GET / PATCH | `/v1/profile` (public, show_recent, shown playlist ids) | access token |
| PUT / DELETE | `/v1/profile/avatar`, `/v1/profile/banner` (raw image, 512 KB / 2 MB) | access token |
| PUT | `/v1/profile/recent {tracks}` (kept only while show_recent is on) | access token |
| GET | `/v1/profiles/{handle}`, `.../playlists/{id}`, `.../images/{sha256}` | access token |

Rate limits use the existing limiter: 10 login starts / 10 min per install id,
handle checks 30 / min. Errors use the existing `ApiError` with codes such as
`handle_taken`, `device_limit`, `unauthorized`, `reauth`.

## 7. Privacy

Stored: handle, passkey public keys, recovery-code hash, device name/platform/
first and last seen, the synced items above, profile settings. Not stored: passwords, IPs, play
history (beyond the opt-in profile songs), audio. The Worker never returns another user's data except a profile; handles are only
checked for availability.

Profiles (17.10.2026): every account may set a picture and a banner, which any
signed-in user sees under `@handle`. Everything else is off until the owner
turns it on in Settings -> Profile & privacy: a public profile shows only the
playlists switched on one by one, and the last 20 songs only with "show
recently played" (turning it off deletes them on the server). Profiles are
never visible without signing in.

Delete account: Settings -> Account -> Delete (type the handle to confirm) ->
`DELETE /v1/account` removes users, passkeys, devices, sync items, the profile and the user's
R2 folder in one go; the handle is held for 30 days. `GET /v1/sync/export` gives a
JSON copy before that. The local library on each device stays.

## 8. Mobile clients (Android / iOS)

Same HTTP API, same JSON. The phone apps use the platform passkey APIs directly
(Android Credential Manager, iOS `ASAuthorizationPlatformPublicKeyCredentialProvider`)
against the `/v1/account/passkey/*` endpoints instead of the browser page, or the
pairing code. Their WebAuthn origins (`android:apk-key-hash:...`) go into the
Worker variable `ACCOUNT_EXTRA_ORIGINS` (comma-separated). The sync engine is a small, documented state machine (outbox + last
rev + LWW merge), so it can be ported or shared via the Rust core (UniFFI) later.
The ASCII style is purely client side; the API carries no UI.

## 9. Native server

`glyph-server-native` gets the same routes later with a SQLite file. v1 enables
accounts only on the Worker; the native binary answers `/v1/account/*` with 404 and
`/v1/health` reports `accounts: false`, so the app hides the page.

## 10. Build plan (v1)

1. `glyph-protocol`: account, device and sync types (optional fields).
2. `glyph-server-core`: `accounts` module with an `AccountStore` trait, an in-memory
   store for tests, WebAuthn ES256 verification (`p256`, `ciborium`), token logic,
   LWW merge, cover cache, routes. Unit tests for every rule above.
3. `glyph-server-worker`: D1 implementation of `AccountStore`, R2 cover store, SQL
   migration in `migrations/0001_accounts.sql`, the login page; `[[d1_databases]]`
   and `[[r2_buckets]]` commented in `wrangler.toml` until they exist.
4. `glyph-core`: account + sync client in `server_client.rs`, outbox and merge in
   a new `sync.rs`, tokens in the OS keychain.
5. App: Settings -> Account (sign in, handle, devices with remove, 5/10 limit,
   pairing code approve, sign out, delete), EN/DE/RU strings.
6. Nothing is deployed. Going live needs, on the glyphmusic account:
   `wrangler d1 create glyph`, `wrangler r2 bucket create glyph-covers`, the
   migration, then a deploy.

## 11. Status (v1, 17.10.2026, deployed)

Built, tested, not deployed:

* Server: `glyph-server-core/src/accounts/` (routes, WebAuthn ES256/RS256,
  tokens, LWW sync, cover cache, login page), D1/R2 adapters in
  `glyph-server-worker/src/accounts.rs`, schema in
  `glyph-server-worker/migrations/0001_accounts.sql`.
* App: `glyph-core/src/account.rs` (session, keychain, sync round),
  `glyph-core/src/sync.rs` (snapshot/diff/apply), account calls in
  `server_client.rs`, Settings -> Account in `src/ui/account.rs`.
* Accounts switch on only when the Worker has the `GLYPH_DB` binding and
  `PUBLIC_ORIGIN`; covers additionally need `GLYPH_COVERS`. Until then
  `/v1/health` reports `accounts: false` and the app shows a hint.

Going live (on the glyphmusic account, after review):

```
npx wrangler d1 create glyph
npx wrangler r2 bucket create glyph-covers
# paste the database_id, uncomment both blocks in wrangler.toml
npx wrangler d1 migrations apply glyph --remote
npx wrangler deploy
```

Deployed 17.10.2026 to the glyphmusic account (D1 `glyph` with migrations 0001
and 0002, R2 `glyph-covers`). Password routes: `GET /v1/account/password/salt/{h}`,
`POST /v1/account/password/register|login`, `PUT /v1/account/password`, page
variants `POST /v1/account/login/password|password_register`, passkey linking
`POST /v1/account/passkey/link` + `/v1/account/login/info`.
