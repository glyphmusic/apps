// splash.rs: the icon's character raster (31 columns x 29 rows) and the
// particles that stream in and assemble it. Shared by the startup splash and
// the live note logo (NoteLogo.svelte: onboarding, Settings > About).

export const ART: string[] = [
  "                         ..+.. ",
  "                       ----#+ .",
  "                  ...-++-++++. ",
  "              ..-----+####+0.  ",
  "           .--.-++++##++++++.  ",
  "         +++++++++++---.. ++.  ",
  "        .+++++++.+....   .#+.  ",
  "        +0...+.++.        ++.  ",
  "        +++...            .+.  ",
  "        .+.               ++.  ",
  "        .+.               .0.  ",
  "        .+.               .+.  ",
  "        .+.               .+.  ",
  "        .+.               .+.  ",
  "        .+.               .+.  ",
  "        .+.               ++.  ",
  "        .+.           ... .+.  ",
  "        .+.         ..++++.+.  ",
  "        .+.        .+.++++0..  ",
  "        .+.       ...-+#+++0+  ",
  "  -++++ .+.       +.+####++++  ",
  " .+.+++#.+.       ..++.0++++.  ",
  ".+++###+0+.        +..+.+++.   ",
  ".+.+###++0+         ....+..    ",
  "..+###+++++           ...      ",
  "..+..0++++.                    ",
  " ....++++.                     ",
  "  ....+..                      ",
  "    ...                        ",
];
/** `p` = pink (highlight), `w` = white. */
export const TINT: string[] = [
  "                         wwwww ",
  "                       wwwwpp w",
  "                  wwwwwwwpwwww ",
  "              wwwwwwwpppppwww  ",
  "           wwwwwwwpppppwwwwww  ",
  "         wwwpwppppppwwwww www  ",
  "        wwwpppwwwwwwww   wwww  ",
  "        wwwwwwwwww        wpw  ",
  "        wwwwww            www  ",
  "        www               www  ",
  "        www               www  ",
  "        www               www  ",
  "        wpw               wpw  ",
  "        www               www  ",
  "        www               www  ",
  "        wpw               wpw  ",
  "        www           www www  ",
  "        www         wwwwwwwww  ",
  "        wpw        wwwpppwwww  ",
  "        wpw       wwwwpppwwww  ",
  "  wwwww www       wwwwpppppww  ",
  " wwwwppwwww       wwwwwwwppww  ",
  "wwwwpppwwww        wwwwwwpww   ",
  "wwwppppwwww         wwwwwww    ",
  "wwwwpppppww           www      ",
  "wwwwwwppwww                    ",
  " wwwwwwwww                     ",
  "  wwwwwww                      ",
  "    www                        ",
];

export const COLS = ART[0].length;
export const ROWS = ART.length;
/** Character cell aspect of the icon raster (height / width). */
export const CELL_ASPECT = 25.2 / 19.85;
/** Number of currents the characters arrive in. */
export const STREAMS = 6;
/** Timeline (seconds). */
export const T_OUT_START = 2.45;
export const T_OUT = 0.45;
export const TOTAL = T_OUT_START + T_OUT;

export interface Particle {
  ch: string;
  pink: boolean;
  /** Target cell. */
  col: number;
  row: number;
  /** Direction it comes from (radians) and its sideways offset in the stream. */
  angle: number;
  lane: number;
  /** Bend of the path (radians), shared by the stream. */
  swirl: number;
  delay: number;
  dur: number;
}

/** Small deterministic hash -> 0..1. */
export function rnd(i: number, salt: number): number {
  let h = (Math.imul(i, 0x9e3779b9) ^ Math.imul(salt, 0x85ebca6b)) >>> 0;
  h ^= h >>> 15;
  h = Math.imul(h, 0x2c1b3c6d) >>> 0;
  h ^= h >>> 12;
  return (h & 0xffff) / 65535;
}

export const easeInOut = (t: number) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);
export const smooth = (t: number) => { t = Math.min(1, Math.max(0, t)); return t * t * (3 - 2 * t); };

export function makeParticles(): Particle[] {
  const out: Particle[] = [];
  let i = 0;
  ART.forEach((line, row) => {
    for (let col = 0; col < line.length; col++) {
      const ch = line[col];
      if (ch === " ") continue;
      const stream = Math.floor(rnd(i, 1) * STREAMS) % STREAMS;
      const base = (stream / STREAMS) * Math.PI * 2 + 0.4;
      const swirl = (stream % 2 === 0 ? 0.9 : -0.9) * (0.75 + 0.5 * rnd(stream, 7));
      out.push({ ch, pink: TINT[row][col] === "p", col, row, angle: base + (rnd(i, 2) - 0.5) * 0.35, lane: rnd(i, 3) - 0.5, swirl, delay: stream * 0.07 + rnd(i, 4) * 0.6, dur: 0.85 + rnd(i, 5) * 0.35 });
      i++;
    }
  });
  return out;
}

/** Trail while moving: fainter copies a little behind (back, alpha). */
export const TRAIL: [number, number][] = [[0.07, 0.18], [0.035, 0.4], [0, 1]];
export const REST: [number, number][] = [[0, 1]];

/** Position on the particle's curved path at progress `e` (0 = far away, 1 = home). */
export function pathAt(p: Particle, e: number, o: { cx: number; cy: number; reach: number; lane: number; left: number; top: number; cellW: number; cellH: number }): [number, number] {
  const a = p.angle, sa = Math.sin(a + Math.PI / 2), ca2 = Math.cos(a + Math.PI / 2);
  const sx = o.cx + o.reach * Math.cos(a) + ca2 * p.lane * o.lane;
  const sy = o.cy + o.reach * Math.sin(a) + sa * p.lane * o.lane;
  // The control point is turned by the stream's swirl: curved currents.
  const c = a + p.swirl;
  const kx = o.cx + o.reach * 0.55 * Math.cos(c), ky = o.cy + o.reach * 0.55 * Math.sin(c);
  const tx = o.left + p.col * o.cellW, ty = o.top + p.row * o.cellH;
  const u = 1 - e;
  return [u * u * sx + 2 * u * e * kx + e * e * tx, u * u * sy + 2 * u * e * ky + e * e * ty];
}

/** CSS colour `rgba()` for the white / pink character at a quantised alpha (12 steps). */
export function charColor(pink: boolean, alpha: number, accent: string): string {
  const step = Math.round(Math.min(1, Math.max(0, alpha)) * 12) / 12;
  if (step <= 0) return "";
  if (!pink) return `rgba(245,245,245,${step})`;
  const m = /^#?([0-9a-f]{6})$/i.exec(accent.trim());
  const n = m ? parseInt(m[1], 16) : 0xff5fd2;
  return `rgba(${(n >> 16) & 255},${(n >> 8) & 255},${n & 255},${step})`;
}
