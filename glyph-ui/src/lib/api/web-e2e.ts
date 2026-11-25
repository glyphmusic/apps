// Browser E2E cryptography compatible with glyph_core::e2e and glyph_core::e2e_sync.
// Uses WebCrypto (HKDF-SHA256, HMAC-SHA256, SHA-256, CSPRNG),
// CompressionStream/DecompressionStream("deflate-raw"), pure-TS XChaCha20-Poly1305
// & X25519, and hash-wasm Argon2id for password KDFs.
import { argon2id } from "hash-wasm";

const enc = new TextEncoder();
const dec = new TextDecoder();

export const SYNC_KIND = {
  LIKED: "liked",
  LIKED_CLEAR: "liked_clear",
  PLAYLIST: "playlist",
  ALBUM: "album",
  FOLLOWED: "followed",
  RECENT: "recent",
   PINNED: "pinned",
  RECENT_LIST: "recent_list",
  SEARCH: "search",
  HIDDEN_SIDEBAR: "hidden_sidebar",
  HIDDEN_ARTIST: "hidden_artist",
  TASTE: "taste",
  FEED_NEW_SEEN: "feed_new_seen",
  LYRICS_OFFSET: "lyrics_offset",
  SETTINGS: "settings",
  STATS: "stats",
  KEYRING: "keyring",
  E2E: "sealed",
  E2E_PLAYLIST: "sealed_playlist",
  /** Whole-value library choices from the apps (glyph_core::sync PREFERENCE). */
  LIBRARY_PREF: "library_pref",
} as const;

export interface SyncItem {
  kind: string;
  key: string;
  value?: any;
  updated_at: number;
  deleted: boolean;
  rev?: number;
}

export function b64url(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
  return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export function unb64url(s: string): Uint8Array | null {
  try {
    const clean = s.trim().replace(/=+$/, "").replace(/-/g, "+").replace(/_/g, "/");
    const bin = atob(clean + "===".slice((clean.length + 3) % 4));
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  } catch {
    return null;
  }
}

export function toHex(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < bytes.length; i++) s += bytes[i].toString(16).padStart(2, "0");
  return s;
}

export function randomBytes(n: number): Uint8Array {
  const b = new Uint8Array(n);
  crypto.getRandomValues(b);
  return b;
}

export async function sha256(data: Uint8Array | string): Promise<Uint8Array> {
  const bytes = typeof data === "string" ? enc.encode(data) : data;
  const buf = await crypto.subtle.digest("SHA-256", bytes as BufferSource);
  return new Uint8Array(buf);
}

// ---- ChaCha20 / HChaCha20 / Poly1305 / XChaCha20-Poly1305 ----

function rotl32(v: number, c: number): number {
  return ((v << c) | (v >>> (32 - c))) >>> 0;
}

function quarterRound(s: Uint32Array, a: number, b: number, c: number, d: number) {
  s[a] = (s[a] + s[b]) >>> 0; s[d] = rotl32(s[d] ^ s[a], 16);
  s[c] = (s[c] + s[d]) >>> 0; s[b] = rotl32(s[b] ^ s[c], 12);
  s[a] = (s[a] + s[b]) >>> 0; s[d] = rotl32(s[d] ^ s[a], 8);
  s[c] = (s[c] + s[d]) >>> 0; s[b] = rotl32(s[b] ^ s[c], 7);
}

function chacha20Rounds(w: Uint32Array) {
  for (let i = 0; i < 10; i++) {
    quarterRound(w, 0, 4, 8, 12);
    quarterRound(w, 1, 5, 9, 13);
    quarterRound(w, 2, 6, 10, 14);
    quarterRound(w, 3, 7, 11, 15);
    quarterRound(w, 0, 5, 10, 15);
    quarterRound(w, 1, 6, 11, 12);
    quarterRound(w, 2, 7, 8, 13);
    quarterRound(w, 3, 4, 9, 14);
  }
}

function u32le(b: Uint8Array, off: number): number {
  return (b[off] | (b[off + 1] << 8) | (b[off + 2] << 16) | (b[off + 3] << 24)) >>> 0;
}

function putU32le(b: Uint8Array, off: number, v: number) {
  b[off] = v & 0xff;
  b[off + 1] = (v >>> 8) & 0xff;
  b[off + 2] = (v >>> 16) & 0xff;
  b[off + 3] = (v >>> 24) & 0xff;
}

function hchacha20(key: Uint8Array, nonce16: Uint8Array): Uint8Array {
  const s = new Uint32Array(16);
  s[0] = 0x61707865; s[1] = 0x3320646e; s[2] = 0x79622d32; s[3] = 0x6b206574;
  for (let i = 0; i < 8; i++) s[4 + i] = u32le(key, i * 4);
  for (let i = 0; i < 4; i++) s[12 + i] = u32le(nonce16, i * 4);
  chacha20Rounds(s);
  const out = new Uint8Array(32);
  for (let i = 0; i < 4; i++) putU32le(out, i * 4, s[i]);
  for (let i = 0; i < 4; i++) putU32le(out, 16 + i * 4, s[12 + i]);
  return out;
}

function chacha20Block(key: Uint8Array, counter: number, nonce12: Uint8Array): Uint8Array {
  const init = new Uint32Array(16);
  init[0] = 0x61707865; init[1] = 0x3320646e; init[2] = 0x79622d32; init[3] = 0x6b206574;
  for (let i = 0; i < 8; i++) init[4 + i] = u32le(key, i * 4);
  init[12] = counter >>> 0;
  for (let i = 0; i < 3; i++) init[13 + i] = u32le(nonce12, i * 4);
  const w = new Uint32Array(init);
  chacha20Rounds(w);
  const out = new Uint8Array(64);
  for (let i = 0; i < 16; i++) putU32le(out, i * 4, (w[i] + init[i]) >>> 0);
  return out;
}

function chacha20Xor(key: Uint8Array, counter: number, nonce12: Uint8Array, input: Uint8Array): Uint8Array {
  const out = new Uint8Array(input.length);
  for (let off = 0, b = counter; off < input.length; off += 64, b++) {
    const ks = chacha20Block(key, b, nonce12);
    const take = Math.min(64, input.length - off);
    for (let i = 0; i < take; i++) out[off + i] = input[off + i] ^ ks[i];
  }
  return out;
}

const POLY_P = (1n << 130n) - 5n;

function bytesToBigLE(b: Uint8Array): bigint {
  let v = 0n;
  for (let i = b.length - 1; i >= 0; i--) v = (v << 8n) | BigInt(b[i]);
  return v;
}

function poly1305Mac(msg: Uint8Array, key32: Uint8Array): Uint8Array {
  const rBytes = new Uint8Array(key32.subarray(0, 16));
  rBytes[3] &= 15; rBytes[7] &= 15; rBytes[11] &= 15; rBytes[15] &= 15;
  rBytes[4] &= 252; rBytes[8] &= 252; rBytes[12] &= 252;
  const r = bytesToBigLE(rBytes);
  const s = bytesToBigLE(key32.subarray(16, 32));
  let acc = 0n;
  for (let i = 0; i < msg.length; i += 16) {
    const slice = msg.subarray(i, Math.min(i + 16, msg.length));
    const n = bytesToBigLE(slice) | (1n << BigInt(slice.length * 8));
    acc = ((acc + n) * r) % POLY_P;
  }
  acc = (acc + s) & ((1n << 128n) - 1n);
  const tag = new Uint8Array(16);
  for (let i = 0; i < 16; i++) {
    tag[i] = Number(acc & 0xffn);
    acc >>= 8n;
  }
  return tag;
}

function poly1305Input(aad: Uint8Array, ct: Uint8Array): Uint8Array {
  const padA = (16 - (aad.length % 16)) % 16;
  const padC = (16 - (ct.length % 16)) % 16;
  const buf = new Uint8Array(aad.length + padA + ct.length + padC + 16);
  buf.set(aad, 0);
  buf.set(ct, aad.length + padA);
  const dv = new DataView(buf.buffer);
  dv.setBigUint64(buf.length - 16, BigInt(aad.length), true);
  dv.setBigUint64(buf.length - 8, BigInt(ct.length), true);
  return buf;
}

export function sealBox(key32: Uint8Array, aad: Uint8Array, plain: Uint8Array): Record<string, any> {
  const nonce = randomBytes(24);
  const subkey = hchacha20(key32, nonce.subarray(0, 16));
  const nonce12 = new Uint8Array(12);
  nonce12.set(nonce.subarray(16, 24), 4);
  const polyKey = chacha20Block(subkey, 0, nonce12).subarray(0, 32);
  const ct = chacha20Xor(subkey, 1, nonce12, plain);
  const tag = poly1305Mac(poly1305Input(aad, ct), polyKey);
  const combined = new Uint8Array(ct.length + 16);
  combined.set(ct, 0);
  combined.set(tag, ct.length);
  return { n: b64url(nonce), ct: b64url(combined) };
}

export function openBox(key32: Uint8Array, aad: Uint8Array, box: any): Uint8Array | null {
  if (!box || typeof box.n !== "string" || typeof box.ct !== "string") return null;
  const nonce = unb64url(box.n);
  const combined = unb64url(box.ct);
  if (!nonce || !combined || nonce.length !== 24 || combined.length < 16) return null;
  const ct = combined.subarray(0, combined.length - 16);
  const tag = combined.subarray(combined.length - 16);
  const subkey = hchacha20(key32, nonce.subarray(0, 16));
  const nonce12 = new Uint8Array(12);
  nonce12.set(nonce.subarray(16, 24), 4);
  const polyKey = chacha20Block(subkey, 0, nonce12).subarray(0, 32);
  const expected = poly1305Mac(poly1305Input(aad, ct), polyKey);
  let diff = 0;
  for (let i = 0; i < 16; i++) diff |= tag[i] ^ expected[i];
  if (diff !== 0) return null;
  return chacha20Xor(subkey, 1, nonce12, ct);
}

// ---- X25519 (Curve25519 Montgomery ladder) ----

const P25519 = (1n << 255n) - 19n;
const A24 = 121665n;

function mod(a: bigint): bigint {
  const r = a % P25519;
  return r < 0n ? r + P25519 : r;
}

function powMod(base: bigint, exp: bigint): bigint {
  let res = 1n;
  let b = mod(base);
  let e = exp;
  while (e > 0n) {
    if (e & 1n) res = mod(res * b);
    b = mod(b * b);
    e >>= 1n;
  }
  return res;
}

export function x25519(scalarBytes: Uint8Array, uBytes: Uint8Array): Uint8Array {
  const k = new Uint8Array(scalarBytes);
  k[0] &= 248;
  k[31] &= 127;
  k[31] |= 64;
  const uArr = new Uint8Array(uBytes);
  uArr[31] &= 127;
  const scalar = bytesToBigLE(k);
  const x1 = bytesToBigLE(uArr);
  let x2 = 1n, z2 = 0n, x3 = x1, z3 = 1n;
  let swap = 0n;
  for (let t = 254; t >= 0; t--) {
    const kt = (scalar >> BigInt(t)) & 1n;
    swap ^= kt;
    if (swap) { [x2, x3] = [x3, x2]; [z2, z3] = [z3, z2]; }
    swap = kt;
    const A = mod(x2 + z2);
    const AA = mod(A * A);
    const B = mod(x2 - z2);
    const BB = mod(B * B);
    const E = mod(AA - BB);
    const C = mod(x3 + z3);
    const D = mod(x3 - z3);
    const DA = mod(D * A);
    const CB = mod(C * B);
    const dacbAdd = mod(DA + CB);
    const dacbSub = mod(DA - CB);
    x3 = mod(dacbAdd * dacbAdd);
    z3 = mod(x1 * mod(dacbSub * dacbSub));
    x2 = mod(AA * BB);
    z2 = mod(E * mod(AA + mod(A24 * E)));
  }
  if (swap) { [x2, x3] = [x3, x2]; [z2, z3] = [z3, z2]; }
  const res = mod(x2 * powMod(z2, P25519 - 2n));
  const out = new Uint8Array(32);
  let v = res;
  for (let i = 0; i < 32; i++) {
    out[i] = Number(v & 0xffn);
    v >>= 8n;
  }
  return out;
}

const BASEPOINT_9 = (() => {
  const b = new Uint8Array(32);
  b[0] = 9;
  return b;
})();

export class DeviceKey {
  readonly secret: Uint8Array;
  readonly pub: Uint8Array;

  constructor(secret: Uint8Array) {
    this.secret = new Uint8Array(secret);
    this.pub = x25519(this.secret, BASEPOINT_9);
  }

  static generate(): DeviceKey {
    return new DeviceKey(randomBytes(32));
  }

  static fromB64(s: string): DeviceKey | null {
    const b = unb64url(s);
    return b && b.length === 32 ? new DeviceKey(b) : null;
  }

  toB64(): string {
    return b64url(this.secret);
  }

  publicB64(): string {
    return b64url(this.pub);
  }
}

// ---- WebCrypto HKDF-SHA256 & length-prefixed HMAC-SHA256 ----

export async function hkdfSha256(ikm: Uint8Array, salt: Uint8Array | string, info: string, len = 32): Promise<Uint8Array> {
  const saltBytes = typeof salt === "string" ? enc.encode(salt) : salt;
  const baseKey = await crypto.subtle.importKey("raw", ikm as BufferSource, "HKDF", false, ["deriveBits"]);
  const bits = await crypto.subtle.deriveBits(
    { name: "HKDF", hash: "SHA-256", salt: saltBytes as BufferSource, info: enc.encode(info) as BufferSource },
    baseKey,
    len * 8,
  );
  return new Uint8Array(bits);
}

export async function hmacLp(key32: Uint8Array, parts: (Uint8Array | string)[]): Promise<Uint8Array> {
  const macKey = await crypto.subtle.importKey("raw", key32 as BufferSource, { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  const encoded = parts.map((p) => (typeof p === "string" ? enc.encode(p) : p));
  const total = encoded.reduce((s, b) => s + 4 + b.length, 0);
  const buf = new Uint8Array(total);
  let off = 0;
  for (const b of encoded) {
    putU32le(buf, off, b.length);
    buf.set(b, off + 4);
    off += 4 + b.length;
  }
  const sig = await crypto.subtle.sign("HMAC", macKey, buf as BufferSource);
  return new Uint8Array(sig);
}

// ---- Raw DEFLATE + bucket padding (glyph_core::e2e pad/unpad) ----

function padPlain(b: Uint8Array): Uint8Array {
  const len = b.length + 4;
  let bucket = 256;
  if (len <= 4096) {
    while (bucket < len) bucket <<= 1;
  } else {
    bucket = Math.ceil(len / 4096) * 4096;
  }
  const out = new Uint8Array(bucket);
  putU32le(out, 0, b.length);
  out.set(b, 4);
  return out;
}

function unpadPlain(b: Uint8Array): Uint8Array | null {
  if (b.length < 4) return null;
  const len = u32le(b, 0);
  if (4 + len > b.length) return null;
  return b.subarray(4, 4 + len);
}

async function streamCollect(s: ReadableStream<Uint8Array>): Promise<Uint8Array> {
  const reader = s.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { value, done } = await reader.read();
    if (value) { chunks.push(value); total += value.length; }
    if (done) break;
  }
  const out = new Uint8Array(total);
  let off = 0;
  for (const c of chunks) { out.set(c, off); off += c.length; }
  return out;
}

async function deflateRaw(data: Uint8Array): Promise<Uint8Array> {
  const cs = new CompressionStream("deflate-raw");
  const w = cs.writable.getWriter();
  void w.write(data as BufferSource).then(() => w.close());
  return streamCollect(cs.readable);
}

async function inflateRaw(data: Uint8Array): Promise<Uint8Array | null> {
  try {
    const ds = new DecompressionStream("deflate-raw");
    const w = ds.writable.getWriter();
    const writeP = w.write(data as BufferSource).then(() => w.close()).catch(() => {});
    const out = await streamCollect(ds.readable);
    await writeP;
    return out;
  } catch {
    return null;
  }
}

// ---- DataKey (matches glyph_core::e2e::DataKey) ----

export class DataKey {
  readonly raw: Uint8Array;
  private kidCache: string | null = null;
  private subCache = new Map<string, Uint8Array>();

  constructor(raw: Uint8Array) {
    this.raw = new Uint8Array(raw);
  }

  static generate(): DataKey {
    return new DataKey(randomBytes(32));
  }

  static fromBytes(b: Uint8Array | null): DataKey | null {
    return b && b.length === 32 ? new DataKey(b) : null;
  }

  static fromB64(s: string): DataKey | null {
    return DataKey.fromBytes(unb64url(s));
  }

  toB64(): string {
    return b64url(this.raw);
  }

  async sub(label: string): Promise<Uint8Array> {
    const hit = this.subCache.get(label);
    if (hit) return hit;
    const derived = await hkdfSha256(this.raw, "glyph-data-key", label, 32);
    this.subCache.set(label, derived);
    return derived;
  }

  async kid(): Promise<string> {
    if (this.kidCache) return this.kidCache;
    const s = await this.sub("glyph-kid-v1");
    this.kidCache = toHex(s.subarray(0, 8));
    return this.kidCache;
  }

  async blind(kind: string, key: string): Promise<string> {
    const s = await this.sub("glyph-index-v1");
    return toHex(await hmacLp(s, [kind, key]));
  }

  async deviceMac(deviceId: string, pk: Uint8Array): Promise<string> {
    const s = await this.sub("glyph-device-v1");
    return toHex(await hmacLp(s, [deviceId, pk]));
  }

  private static itemAad(encKind: string, blinded: string): Uint8Array {
    return enc.encode(`glyph-e2e-v1\0${encKind}\0${blinded}`);
  }

  async sealItem(item: SyncItem): Promise<SyncItem> {
    const encKind = item.kind === SYNC_KIND.PLAYLIST ? SYNC_KIND.E2E_PLAYLIST : SYNC_KIND.E2E;
    const blinded = await this.blind(item.kind, item.key);
    let value: any = undefined;
    if (item.value !== undefined && item.value !== null && !item.deleted) {
      const plain = enc.encode(JSON.stringify({ k: item.kind, key: item.key, v: item.value }));
      const compressed = await deflateRaw(plain);
      const padded = padPlain(compressed);
      const sub = await this.sub("glyph-item-v1");
      const sealed = sealBox(sub, DataKey.itemAad(encKind, blinded), padded);
      sealed.v = 1;
      sealed.kid = await this.kid();
      value = sealed;
    }
    return {
      kind: encKind,
      key: blinded,
      deleted: value === undefined,
      value,
      updated_at: item.updated_at,
      rev: item.rev,
    };
  }

  async openItem(
    item: SyncItem,
    lookup: (blinded: string) => [string, string] | null,
  ): Promise<{ ok: true; item: SyncItem | null } | { ok: false; otherKid?: string }> {
    if (item.deleted || item.value == null) {
      const found = lookup(item.key);
      if (!found) return { ok: true, item: null };
      return {
        ok: true,
        item: { kind: found[0], key: found[1], value: undefined, updated_at: item.updated_at, deleted: true, rev: item.rev },
      };
    }
    const v = item.value;
    const ourKid = await this.kid();
    if (v?.kid !== ourKid) {
      return { ok: false, otherKid: typeof v?.kid === "string" ? v.kid : "" };
    }
    const sub = await this.sub("glyph-item-v1");
    const padded = openBox(sub, DataKey.itemAad(item.kind, item.key), v);
    if (!padded) return { ok: false };
    const deflated = unpadPlain(padded);
    if (!deflated) return { ok: false };
    const plainBytes = await inflateRaw(deflated);
    if (!plainBytes) return { ok: false };
    try {
      const inner = JSON.parse(dec.decode(plainBytes));
      if (typeof inner?.k !== "string" || typeof inner?.key !== "string") return { ok: false };
      const expectedBlind = await this.blind(inner.k, inner.key);
      if (expectedBlind !== item.key || (inner.k === SYNC_KIND.PLAYLIST) !== (item.kind === SYNC_KIND.E2E_PLAYLIST)) {
        return { ok: false };
      }
      return {
        ok: true,
        item: {
          kind: inner.k,
          key: inner.key,
          value: inner.v,
          updated_at: item.updated_at,
          deleted: false,
          rev: item.rev,
        },
      };
    } catch {
      return { ok: false };
    }
  }
}

// ---- Device Envelopes (`env:<device_id>`) ----

function envAad(deviceId: string, kid: string): Uint8Array {
  return enc.encode(`glyph-env-v1\0${deviceId}\0${kid}`);
}

function isZero32(b: Uint8Array): boolean {
  let acc = 0;
  for (let i = 0; i < b.length; i++) acc |= b[i];
  return acc === 0;
}

export async function sealToDevice(deviceId: string, pkB64: string, k: DataKey, by: string): Promise<Record<string, any> | null> {
  const pk = unb64url(pkB64);
  if (!pk || pk.length !== 32) return null;
  const eph = DeviceKey.generate();
  const shared = x25519(eph.secret, pk);
  if (isZero32(shared)) return null;
  const salt = new Uint8Array(64);
  salt.set(eph.pub, 0);
  salt.set(pk, 32);
  const kek = await hkdfSha256(shared, salt, "glyph-env-v1", 32);
  const kid = await k.kid();
  const v = sealBox(kek, envAad(deviceId, kid), k.raw);
  v.epk = eph.publicB64();
  v.kid = kid;
  v.by = by;
  return v;
}

export async function openFromDevice(deviceId: string, dk: DeviceKey, v: any): Promise<DataKey | null> {
  if (!v || typeof v.kid !== "string" || typeof v.epk !== "string") return null;
  const epk = unb64url(v.epk);
  if (!epk || epk.length !== 32) return null;
  const shared = x25519(dk.secret, epk);
  if (isZero32(shared)) return null;
  const salt = new Uint8Array(64);
  salt.set(epk, 0);
  salt.set(dk.pub, 32);
  const kek = await hkdfSha256(shared, salt, "glyph-env-v1", 32);
  const opened = openBox(kek, envAad(deviceId, v.kid), v);
  const key = DataKey.fromBytes(opened);
  if (!key || (await key.kid()) !== v.kid) return null;
  return key;
}

// ---- Password KDFs via hash-wasm Argon2id ----

/** glyph_protocol::password_kdf (M=19456 KiB, T=2, P=1, len=32). Returns base64url key. */
export async function deriveLoginKey(password: string, saltB64: string): Promise<string> {
  const salt = unb64url(saltB64);
  if (!salt || salt.length < 8) throw new Error("invalid salt");
  const key: Uint8Array = await argon2id({
    password,
    salt,
    parallelism: 1,
    iterations: 2,
    memorySize: 19456,
    hashLength: 32,
    outputType: "binary",
  });
  return b64url(key);
}

/** glyph_protocol::vault_kdf (M=65536 KiB, T=3, P=1, len=32, secret="glyph-vault-v2"). */
export async function derivePasswordKey(password: string, accountId: string): Promise<Uint8Array> {
  const fullSalt = await sha256(`glyph-vault-v2:${accountId}`);
  const salt = fullSalt.subarray(0, 16);
  const key: Uint8Array = await argon2id({
    password,
    salt,
    secret: enc.encode("glyph-vault-v2"),
    parallelism: 1,
    iterations: 3,
    memorySize: 65536,
    hashLength: 32,
    outputType: "binary",
  });
  return key;
}

export async function suspiciousSignInSalt(saltB64: string, accountId: string): Promise<boolean> {
  const salt = unb64url(saltB64);
  if (!salt || salt.length !== 16) return false;
  const v1 = (await sha256(`glyph-settings-v1:${accountId}`)).subarray(0, 16);
  const v2 = (await sha256(`glyph-vault-v2:${accountId}`)).subarray(0, 16);
  const eq = (a: Uint8Array, b: Uint8Array) => a.every((x, i) => x === b[i]);
  return eq(salt, v1) || eq(salt, v2);
}

function pwAad(accountId: string, kid: string): Uint8Array {
  return enc.encode(`glyph-keyring-pw-v1\0${accountId}\0${kid}`);
}

export async function wrapWithPassword(pwKey: Uint8Array, accountId: string, k: DataKey): Promise<Record<string, any>> {
  const kid = await k.kid();
  return sealBox(pwKey, pwAad(accountId, kid), k.raw);
}

export async function unwrapWithPassword(pwKey: Uint8Array, accountId: string, kid: string, v: any): Promise<DataKey | null> {
  const plain = openBox(pwKey, pwAad(accountId, kid), v);
  const k = DataKey.fromBytes(plain);
  if (!k || (await k.kid()) !== kid) return null;
  return k;
}

// ---- Pairing code helper ----

const CODE_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

export function splitPairCode(raw: string): [string, string | null] | null {
  const c = raw
    .toUpperCase()
    .replace(/[^0-9A-Z]/g, "");
  for (let i = 0; i < c.length; i++) {
    if (!CODE_ALPHABET.includes(c[i])) return null;
  }
  if (c.length === 8) return [`${c.slice(0, 4)}-${c.slice(4, 8)}`, null];
  if (c.length === 12) return [`${c.slice(0, 4)}-${c.slice(4, 8)}`, c.slice(8, 12)];
  return null;
}
