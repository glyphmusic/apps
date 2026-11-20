// Playlist share codes in the browser, the same format as glyph-core's
// share.rs: `glyph:playlist:` + base64url(raw deflate(JSON)). The web player
// makes one before it asks the server for a short link (`POST /v1/share`
// only takes a code), and reads one back to open a shared `glyph.my/p/<id>`
// link right in the web player.
import type { Track } from "./types";

export const PREFIX = "glyph:playlist:";
/** Bigger codes are refused (share.rs MAX_JSON). */
const MAX_JSON = 4 * 1024 * 1024;

type Song = { t: string; a: string; al?: string; s: string; i: string; d?: number; isrc?: string; x?: boolean };

function b64url(bytes: Uint8Array): string {
  let bin = "";
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function unb64url(s: string): Uint8Array {
  const std = s.replace(/-/g, "+").replace(/_/g, "/").replace(/=+$/, "");
  const bin = atob(std + "=".repeat((4 - (std.length % 4)) % 4));
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/** Runs `bytes` through a (de)compression stream. `lenient`: a stream that
 *  ends early keeps what came out (flate2 in share.rs reads such codes too). */
async function pipe(bytes: Uint8Array, stream: CompressionStream | DecompressionStream, limit = Infinity, lenient = false): Promise<Uint8Array> {
  const writer = stream.writable.getWriter();
  const reader = stream.readable.getReader();
  const fed = writer.write(bytes as BufferSource).then(() => writer.close()).catch(() => {});
  const parts: Uint8Array[] = [];
  let size = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.length;
      if (size > limit) { void reader.cancel().catch(() => {}); throw new RangeError("playlist code too large"); }
      parts.push(value);
    }
  } catch (e) {
    if (!lenient || !size || e instanceof RangeError) throw e;
  }
  await fed;
  const out = new Uint8Array(size);
  let at = 0;
  for (const p of parts) { out.set(p, at); at += p.length; }
  return out;
}

/** The code for `name` and `tracks` (what `share.rs encode` makes). */
export async function encodeShare(name: string, tracks: Track[]): Promise<string> {
  const t: Song[] = tracks.map((x) => {
    const s: Song = { t: x.title, a: x.artist, s: x.source, i: x.id };
    if (x.album) s.al = x.album;
    if (x.duration_ms != null) s.d = x.duration_ms;
    if (x.isrc) s.isrc = x.isrc;
    if (x.explicit) s.x = true;
    return s;
  });
  const json = new TextEncoder().encode(JSON.stringify({ v: 1, n: name, t }));
  return PREFIX + b64url(await pipe(json, new CompressionStream("deflate-raw")));
}

/** Name and songs of a code; throws on a broken one. */
export async function decodeShare(code: string): Promise<{ name: string; tracks: Track[] }> {
  const body = code.trim();
  if (!body.startsWith(PREFIX)) throw new Error("not a Glyph playlist code");
  const json = await pipe(unb64url(body.slice(PREFIX.length)), new DecompressionStream("deflate-raw"), MAX_JSON, true);
  const c = JSON.parse(new TextDecoder().decode(json)) as { n?: string; t?: Song[] };
  const tracks: Track[] = (c.t ?? [])
    .filter((s) => typeof s?.t === "string" && s.t.trim() && typeof s.a === "string")
    .map((s) => {
      const t: Track = { id: String(s.i ?? ""), source: s.s as Track["source"], title: s.t.trim().slice(0, 300), artist: s.a.trim().slice(0, 300) };
      if (typeof s.al === "string" && s.al.trim()) t.album = s.al.trim().slice(0, 300);
      if (typeof s.d === "number" && s.d > 0) t.duration_ms = s.d;
      if (typeof s.isrc === "string" && s.isrc) t.isrc = s.isrc;
      if (s.x) t.explicit = true;
      return t;
    });
  const name = (c.n ?? "").trim().slice(0, 100);
  return { name: name || "Playlist", tracks };
}

/** Id of a short share link (`https://glyph.my/p/AbC123xy`), else null. */
export function shareLinkId(text: string): string | null {
  const m = /^(?:https?:\/\/(?:api\.)?glyph\.my|glyph:\/)\/p\/([A-Za-z0-9]{8}|[A-Za-z0-9]{12})\/?(?:[?#].*)?$/i.exec(text.trim());
  return m ? m[1] : null;
}
