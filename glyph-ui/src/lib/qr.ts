// A small QR code encoder (byte mode, error correction level M, versions
// 1..20) for the jam invite link; the same job `qrcode::QrCode` does in the
// Rust app (ui/jam.rs qr). Follows the ISO 18004 layout (function patterns,
// Reed-Solomon over GF(256), 8 masks scored by the usual run / block /
// finder / balance rules).

/** Error correction codewords per block and number of blocks, level M, by version (index 0 unused). */
const ECC_PER_BLOCK = [-1, 10, 16, 26, 18, 24, 16, 18, 22, 22, 26, 30, 22, 22, 24, 24, 28, 28, 26, 26, 26];
const BLOCKS = [-1, 1, 1, 1, 2, 2, 4, 4, 4, 5, 5, 5, 8, 9, 9, 10, 10, 11, 13, 14, 16];
const MAX_VERSION = 20;

function rawModules(ver: number): number {
  let r = (16 * ver + 128) * ver + 64;
  if (ver >= 2) {
    const n = Math.floor(ver / 7) + 2;
    r -= (25 * n - 10) * n - 55;
    if (ver >= 7) r -= 36;
  }
  return r;
}
const dataCodewords = (ver: number) => Math.floor(rawModules(ver) / 8) - ECC_PER_BLOCK[ver] * BLOCKS[ver];

function rsMul(x: number, y: number): number {
  let z = 0;
  for (let i = 7; i >= 0; i--) { z = (z << 1) ^ ((z >>> 7) * 0x11d); z ^= ((y >>> i) & 1) * x; }
  return z;
}
function rsDivisor(degree: number): number[] {
  const r = new Array<number>(degree).fill(0);
  r[degree - 1] = 1;
  let root = 1;
  for (let i = 0; i < degree; i++) {
    for (let j = 0; j < r.length; j++) { r[j] = rsMul(r[j], root); if (j + 1 < r.length) r[j] ^= r[j + 1]; }
    root = rsMul(root, 2);
  }
  return r;
}
function rsRemainder(data: number[], div: number[]): number[] {
  const r = div.map(() => 0);
  for (const b of data) {
    const f = b ^ r.shift()!;
    r.push(0);
    div.forEach((c, i) => { r[i] ^= rsMul(c, f); });
  }
  return r;
}

function utf8(s: string): number[] { return [...new TextEncoder().encode(s)]; }

function dataBits(bytes: number[], ver: number): number[] {
  const bits: number[] = [];
  const put = (v: number, n: number) => { for (let i = n - 1; i >= 0; i--) bits.push((v >>> i) & 1); };
  put(0b0100, 4);
  put(bytes.length, ver <= 9 ? 8 : 16);
  for (const b of bytes) put(b, 8);
  const cap = dataCodewords(ver) * 8;
  put(0, Math.min(4, cap - bits.length));
  put(0, (8 - (bits.length % 8)) % 8);
  for (let pad = 0xec; bits.length < cap; pad ^= 0xec ^ 0x11) put(pad, 8);
  return bits;
}

function interleave(data: number[], ver: number): number[] {
  const nb = BLOCKS[ver], el = ECC_PER_BLOCK[ver], raw = Math.floor(rawModules(ver) / 8);
  const shortBlocks = nb - (raw % nb), shortLen = Math.floor(raw / nb);
  const div = rsDivisor(el);
  const blocks: number[][] = [];
  for (let i = 0, k = 0; i < nb; i++) {
    const dat = data.slice(k, k + shortLen - el + (i < shortBlocks ? 0 : 1));
    k += dat.length;
    const ecc = rsRemainder(dat, div);
    if (i < shortBlocks) dat.push(0);
    blocks.push(dat.concat(ecc));
  }
  const out: number[] = [];
  for (let i = 0; i < blocks[0].length; i++) blocks.forEach((b, j) => { if (i !== shortLen - el || j >= shortBlocks) out.push(b[i]); });
  return out;
}

const bit = (x: number, i: number) => ((x >>> i) & 1) !== 0;

/** The QR code of `text` as rows of dark (true) / light modules, or null when it does not fit. */
export function qrMatrix(text: string): boolean[][] | null {
  const bytes = utf8(text);
  let ver = 1;
  for (; ver <= MAX_VERSION; ver++) if (4 + (ver <= 9 ? 8 : 16) + bytes.length * 8 <= dataCodewords(ver) * 8) break;
  if (ver > MAX_VERSION) return null;

  const bits = dataBits(bytes, ver);
  const data: number[] = [];
  for (let i = 0; i < bits.length; i += 8) data.push(parseInt(bits.slice(i, i + 8).join(""), 2));
  const words = interleave(data, ver);

  const size = ver * 4 + 17;
  const m: boolean[][] = Array.from({ length: size }, () => new Array<boolean>(size).fill(false));
  const fn: boolean[][] = Array.from({ length: size }, () => new Array<boolean>(size).fill(false));
  const set = (x: number, y: number, dark: boolean) => { m[y][x] = dark; fn[y][x] = true; };

  // function patterns
  for (let i = 0; i < size; i++) { set(6, i, i % 2 === 0); set(i, 6, i % 2 === 0); }
  const finder = (cx: number, cy: number) => {
    for (let dy = -4; dy <= 4; dy++) for (let dx = -4; dx <= 4; dx++) {
      const d = Math.max(Math.abs(dx), Math.abs(dy)), x = cx + dx, y = cy + dy;
      if (x >= 0 && x < size && y >= 0 && y < size) set(x, y, d !== 2 && d !== 4);
    }
  };
  finder(3, 3); finder(size - 4, 3); finder(3, size - 4);
  const pos: number[] = [];
  if (ver > 1) {
    const n = Math.floor(ver / 7) + 2, step = ver === 32 ? 26 : Math.ceil((ver * 4 + 4) / (n * 2 - 2)) * 2;
    pos.push(6);
    for (let p = size - 7, c = 1; c < n; p -= step, c++) pos.splice(1, 0, p);
  }
  pos.forEach((cy, i) => pos.forEach((cx, j) => {
    if ((i === 0 && j === 0) || (i === 0 && j === pos.length - 1) || (i === pos.length - 1 && j === 0)) return;
    for (let dy = -2; dy <= 2; dy++) for (let dx = -2; dx <= 2; dx++) set(cx + dx, cy + dy, Math.max(Math.abs(dx), Math.abs(dy)) !== 1);
  }));
  const formatBits = (mask: number) => {
    const d = mask; // level M = 0b00
    let rem = d;
    for (let i = 0; i < 10; i++) rem = (rem << 1) ^ ((rem >>> 9) * 0x537);
    const b = ((d << 10) | rem) ^ 0x5412;
    for (let i = 0; i <= 5; i++) set(8, i, bit(b, i));
    set(8, 7, bit(b, 6)); set(8, 8, bit(b, 7)); set(7, 8, bit(b, 8));
    for (let i = 9; i < 15; i++) set(14 - i, 8, bit(b, i));
    for (let i = 0; i < 8; i++) set(size - 1 - i, 8, bit(b, i));
    for (let i = 8; i < 15; i++) set(8, size - 15 + i, bit(b, i));
    set(8, size - 8, true);
  };
  formatBits(0);
  if (ver >= 7) {
    let rem = ver;
    for (let i = 0; i < 12; i++) rem = (rem << 1) ^ ((rem >>> 11) * 0x1f25);
    const b = (ver << 12) | rem;
    for (let i = 0; i < 18; i++) { const a = size - 11 + (i % 3), c = Math.floor(i / 3); set(a, c, bit(b, i)); set(c, a, bit(b, i)); }
  }

  // codewords, zig-zag from the bottom right
  let k = 0;
  for (let right = size - 1; right >= 1; right -= 2) {
    if (right === 6) right = 5;
    for (let vert = 0; vert < size; vert++) for (let j = 0; j < 2; j++) {
      const x = right - j, up = ((right + 1) & 2) === 0, y = up ? size - 1 - vert : vert;
      if (!fn[y][x] && k < words.length * 8) { m[y][x] = bit(words[k >>> 3], 7 - (k & 7)); k++; }
    }
  }

  const mask = (n: number) => {
    for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) {
      const inv = [(x + y) % 2 === 0, y % 2 === 0, x % 3 === 0, (x + y) % 3 === 0, (Math.floor(x / 3) + Math.floor(y / 2)) % 2 === 0,
        ((x * y) % 2) + ((x * y) % 3) === 0, (((x * y) % 2) + ((x * y) % 3)) % 2 === 0, (((x + y) % 2) + ((x * y) % 3)) % 2 === 0][n];
      if (!fn[y][x] && inv) m[y][x] = !m[y][x];
    }
  };
  const penalty = (): number => {
    let p = 0;
    const lines: string[] = [];
    for (let y = 0; y < size; y++) lines.push(m[y].map((d) => (d ? "1" : "0")).join(""));
    for (let x = 0; x < size; x++) lines.push(m.map((r) => (r[x] ? "1" : "0")).join(""));
    for (const l of lines) {
      for (const run of l.match(/0+|1+/g) ?? []) if (run.length >= 5) p += 3 + (run.length - 5);
      p += 40 * ((l.match(/(?=10111010000|00001011101)/g) ?? []).length);
    }
    for (let y = 0; y < size - 1; y++) for (let x = 0; x < size - 1; x++) if (m[y][x] === m[y][x + 1] && m[y][x] === m[y + 1][x] && m[y][x] === m[y + 1][x + 1]) p += 3;
    const dark = m.reduce((a, r) => a + r.filter(Boolean).length, 0), total = size * size;
    p += Math.floor(Math.abs(dark * 20 - total * 10) / total) * 10;
    return p;
  };
  let best = 0, bestScore = Infinity;
  for (let n = 0; n < 8; n++) {
    mask(n); formatBits(n);
    const s = penalty();
    if (s < bestScore) { best = n; bestScore = s; }
    mask(n); // undo
  }
  mask(best); formatBits(best);
  return m;
}

/** SVG path data of the dark modules (one rectangle per run in a row). */
export function qrPath(mx: boolean[][]): string {
  let d = "";
  mx.forEach((row, y) => {
    for (let x = 0; x < row.length; ) {
      if (!row[x]) { x++; continue; }
      const s = x;
      while (x < row.length && row[x]) x++;
      d += `M${s} ${y}h${x - s}v1h${s - x}z`;
    }
  });
  return d;
}
