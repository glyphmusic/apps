// The whole background (glow.ts) drawn by the graphics card: Settings >
// Appearance > Animations "GPU". One small pass computes the glow field
// (one sample per 8 x 8 px, same shapes, noise, ramps and fades as the CPU
// loop in glow.ts), a second stretches it into soft light over the window,
// and the ASCII characters and titlebar dots (glow-gl.ts) go on top. The CPU
// only hands over a few numbers per frame. Needs WebGL 2 (integer maths for
// the noise); without it glow.ts uses the CPU field.
import { VERT, FRAG, fontBytes } from "./glow-gl";

type Rgb = [number, number, number];
type Ramp = [Rgb, Rgb, Rgb, Rgb];

/** One frame's numbers, worked out by glow.ts. */
export interface GpuFrame {
  W: number; H: number; iw: number; ih: number; pitch: number; aspect: number;
  /** 0 strahl, 1 band, 2 sonnenaufgang, 3 horizont. */
  preset: number; prevPreset: number;
  /** 1 when there is no shape to fade from. */
  presetT: number;
  cool: Ramp; warm: Ramp; prevCool: Ramp; prevWarm: Ramp;
  dx: number; dy: number; size: number; gain: number;
  t: number; sh: number; level: number;
  density: number; threshold: number; dotLevel: number; blend: number;
  /** Characters and dots on top (off in reduced motion). */
  ascii: boolean; dots: boolean;
}

const VERT3 = `#version 300 es
in vec2 aPos;
void main() { gl_Position = vec4(aPos, 0.0, 1.0); }
`;

// glow.ts sampleInto / sampleMixRgb / noise / the draw loop, line by line.
const FIELD = `#version 300 es
precision highp float;
precision highp int;
uniform vec2 uFieldSize;
uniform float uAspect;
uniform int uPreset;
uniform int uPrevPreset;
uniform float uPresetT;
uniform vec4 uMotion;
uniform float uT;
uniform float uSh;
uniform float uLevel;
uniform vec3 uCool[4];
uniform vec3 uWarm[4];
uniform vec3 uPCool[4];
uniform vec3 uPWarm[4];
layout(location = 0) out vec4 oField;
layout(location = 1) out vec4 oGlow;

float gauss(float x) { return exp(-x * x); }
float sm(float a, float b, float x) { float t = clamp((x - a) / (b - a), 0.0, 1.0); return t * t * (3.0 - 2.0 * t); }
float fade(float t) { return t * t * (3.0 - 2.0 * t); }

float hash(float x, float y, float z) {
  uint h = uint(int(x)) * 374761393u + uint(int(y)) * 668265263u + uint(int(z)) * 1274126177u;
  h = (h ^ (h >> 13u)) * 1274126177u;
  h = h ^ (h >> 16u);
  return float(h) / 4294967295.0 * 2.0 - 1.0;
}

float vnoise(float x, float y, float z) {
  float xi = floor(x), yi = floor(y), zi = floor(z);
  float xf = fade(x - xi), yf = fade(y - yi), zf = fade(z - zi);
  float x1 = xi + 1.0, y1 = yi + 1.0, z1 = zi + 1.0;
  float a = hash(xi, yi, zi), b = hash(x1, yi, zi), c = hash(xi, y1, zi), d = hash(x1, y1, zi);
  float e = hash(xi, yi, z1), f = hash(x1, yi, z1), g = hash(xi, y1, z1), h = hash(x1, y1, z1);
  float ab = a + (b - a) * xf, cd = c + (d - c) * xf, ef = e + (f - e) * xf, gh = g + (h - g) * xf;
  float lo = ab + (cd - ab) * yf, hi = ef + (gh - ef) * yf;
  return lo + (hi - lo) * zf;
}

// x: brightness, y: warm / cool mix.
vec2 shape(int p, float x, float y) {
  x -= uMotion.x; y -= uMotion.y;
  float size = uMotion.z;
  if (p == 0) {
    float ax = -0.05 * uAspect, ay = 1.08, bx = uAspect * 1.02, by = -0.08;
    float dx = bx - ax, dy = by - ay, len = length(vec2(dx, dy)), ux = dx / len, uy = dy / len;
    float px = x - ax, py = y - ay, s = (px * ux + py * uy) / len, d = abs(px * uy - py * ux);
    float sc = clamp(s, 0.0, 1.0);
    float w = (0.02 + 0.17 * pow(sc, 1.15)) * size;
    float along = sm(-0.05, 0.25, s) * (0.55 + 0.45 * sc);
    return vec2((gauss(d / w) + gauss(d / (w * 3.0)) * 0.26) * along, sm(0.35, 1.0, s));
  }
  if (p == 1) {
    float u = (x / uAspect) * 2.0 - 1.0, profile = pow(max(0.0, 1.0 - u * u), 0.85);
    float w = (0.025 + 0.2 * profile) * size, d = y - 0.52;
    return vec2(gauss(d / w) * (0.35 + 0.65 * profile) + gauss(d / (w * 3.5 + 0.04)) * 0.32 * (0.3 + 0.7 * profile), 0.5);
  }
  if (p == 2) {
    float cx = uAspect * 0.5, cy = 1.52, r = 0.78 * size * min(1.7, max(1.0, uAspect)) / 1.35;
    float e = length(vec2(x - cx, y - cy)) - r;
    float rim = gauss(e / 0.06), halo = gauss(max(0.0, e) / 0.38) * 0.6, inr = e < 0.0 ? 0.75 * exp(e / 0.55) : 0.0;
    return vec2(max(rim, max(halo, inr)), 1.0 - clamp(y, 0.0, 1.0));
  }
  float sky = gauss(y / (0.62 * size)) * 0.98;
  float cx = uAspect * 0.5, cy = 1.95, r = 1.32 / max(0.5, size) * min(1.7, max(1.0, uAspect)) / 1.35 + 0.05;
  float dist = length(vec2(x - cx, y - cy)), mask = sm(r - 0.02, r + 0.22, dist);
  return vec2(sky * mask + gauss((dist - r) / 0.045) * 0.5, 1.0 - mask);
}

vec3 mixRgb(bool prev, float b, float s) {
  float vc = clamp(min(b, 1.0), 0.0, 1.0) * 3.0;
  int ic = min(2, int(vc));
  float fc = vc - float(ic);
  vec3 c, w;
  if (prev) {
    c = uPCool[ic] + (uPCool[ic + 1] - uPCool[ic]) * fc;
    w = uPWarm[ic] + (uPWarm[ic + 1] - uPWarm[ic]) * fc;
  } else {
    c = uCool[ic] + (uCool[ic + 1] - uCool[ic]) * fc;
    w = uWarm[ic] + (uWarm[ic + 1] - uWarm[ic]) * fc;
  }
  return c + (w - c) * s;
}

void main() {
  // Texture row 0 is the top row, like the CPU field.
  float x = gl_FragCoord.x / uFieldSize.x * uAspect;
  float y = gl_FragCoord.y / uFieldSize.y;
  vec2 bs = shape(uPreset, x, y);
  float b = bs.x;
  vec3 c = mixRgb(false, b, bs.y);
  if (uPresetT < 1.0) {
    vec2 ps = shape(uPrevPreset, x, y);
    vec3 pc = mixRgb(true, ps.x, ps.y);
    b = ps.x + (b - ps.x) * uPresetT;
    c = pc + (c - pc) * uPresetT;
  }
  float n = vnoise(x * 1.7, y * 1.7, uT * 0.045) * uSh;
  b = max(0.0, (b + n * min(b, 1.0) * 1.4) * uMotion.w);
  float k = sm(0.02, 1.1, b) * uLevel * 0.5;
  // Colour + brightness / 2 for the characters (as glow-gl.ts expects), and the light itself.
  oField = vec4(c, min(b * 0.5, 1.0));
  oGlow = vec4(c * k, 1.0);
}
`;

// The low-res light stretched over the window, smooth like the browser
// stretches the CPU's small canvas.
const GLOW = `#version 300 es
precision highp float;
uniform sampler2D uGlow;
uniform vec2 uRes;
out vec4 o;
void main() {
  vec2 uv = vec2(gl_FragCoord.x / uRes.x, 1.0 - gl_FragCoord.y / uRes.y);
  o = vec4(texture(uGlow, uv).rgb, 1.0);
}
`;

const ASCII_UNIFORMS = ["uField", "uFont", "uRes", "uFieldSize", "uPitch", "uDensity", "uThreshold", "uLevel", "uDotLevel", "uBlend", "uDots"];
const FIELD_UNIFORMS = ["uFieldSize", "uAspect", "uPreset", "uPrevPreset", "uPresetT", "uMotion", "uT", "uSh", "uLevel", "uCool", "uWarm", "uPCool", "uPWarm"];

let probed: boolean | null = null;

export class FieldGpu {
  private gl: WebGL2RenderingContext;
  private fieldProg!: WebGLProgram;
  private glowProg!: WebGLProgram;
  private asciiProg!: WebGLProgram;
  private fbo!: WebGLFramebuffer;
  private fieldTex!: WebGLTexture;
  private glowTex!: WebGLTexture;
  private fontTex!: WebGLTexture;
  private buf!: WebGLBuffer;
  private uf: Record<string, WebGLUniformLocation | null> = {};
  private ug: Record<string, WebGLUniformLocation | null> = {};
  private ua: Record<string, WebGLUniformLocation | null> = {};
  private fw = 0; private fh = 0;
  private ramps = new Float32Array(12);
  /** The GPU dropped the context (driver reset, app in the background on phones). */
  lost = false;

  /**
   * Whether this device can draw the background on the GPU. Tried once on a
   * spare canvas: a canvas that got a WebGL 2 context cannot fall back to a
   * 2D one any more.
   */
  static supported(font: number[][]): boolean {
    if (probed != null) return probed;
    probed = false;
    try {
      const c = document.createElement("canvas");
      c.width = c.height = 4;
      const gl = c.getContext("webgl2", { antialias: false, depth: false, stencil: false });
      if (gl) {
        const f = new FieldGpu(c, gl, font, () => {});
        f.destroy();
        probed = true;
      }
    } catch (e) {
      console.warn("[glow] GPU background unavailable, using the CPU:", e);
    }
    return probed === true;
  }

  /** null without WebGL 2. `onChange` runs when the context is lost or back. */
  static create(canvas: HTMLCanvasElement, font: number[][], onChange: () => void): FieldGpu | null {
    if (!FieldGpu.supported(font)) return null;
    try {
      const gl = canvas.getContext("webgl2", { alpha: false, antialias: false, depth: false, stencil: false, preserveDrawingBuffer: false, powerPreference: "low-power" });
      if (!gl) return null;
      return new FieldGpu(canvas, gl, font, onChange);
    } catch (e) {
      console.warn("[glow] GPU background unavailable, using the CPU:", e);
      return null;
    }
  }

  private constructor(private canvas: HTMLCanvasElement, gl: WebGL2RenderingContext, private font: number[][], private onChange: () => void) {
    this.gl = gl;
    canvas.addEventListener("webglcontextlost", this.onLost);
    canvas.addEventListener("webglcontextrestored", this.onRestored);
    this.setup();
  }

  private onLost = (e: Event) => {
    e.preventDefault();
    this.lost = true;
    this.onChange();
  };

  private onRestored = () => {
    try {
      this.setup();
      this.fw = 0; this.fh = 0;
      this.lost = false;
    } catch (e) {
      console.warn("[glow] GPU background could not be restored:", e);
    }
    this.onChange();
  };

  private program(vs: string, fs: string): WebGLProgram {
    const gl = this.gl;
    const sh = (type: number, src: string) => {
      const s = gl.createShader(type)!;
      gl.shaderSource(s, src);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s) ?? "shader");
      return s;
    };
    const p = gl.createProgram()!;
    gl.attachShader(p, sh(gl.VERTEX_SHADER, vs));
    gl.attachShader(p, sh(gl.FRAGMENT_SHADER, fs));
    // All three passes share one full-window triangle at attribute 0.
    gl.bindAttribLocation(p, 0, "aPos");
    gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p) ?? "link");
    return p;
  }

  private texture(filter: number): WebGLTexture {
    const gl = this.gl;
    const t = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, t);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return t;
  }

  private setup() {
    const gl = this.gl;
    this.fieldProg = this.program(VERT3, FIELD);
    this.glowProg = this.program(VERT3, GLOW);
    this.asciiProg = this.program(VERT, FRAG);
    for (const n of FIELD_UNIFORMS) this.uf[n] = gl.getUniformLocation(this.fieldProg, n);
    this.ug.uGlow = gl.getUniformLocation(this.glowProg, "uGlow");
    this.ug.uRes = gl.getUniformLocation(this.glowProg, "uRes");
    for (const n of ASCII_UNIFORMS) this.ua[n] = gl.getUniformLocation(this.asciiProg, n);

    this.buf = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.buf);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);

    // Units: 0 field (sharp cells for the characters), 1 font, 2 light (smooth).
    gl.activeTexture(gl.TEXTURE0);
    this.fieldTex = this.texture(gl.NEAREST);
    gl.activeTexture(gl.TEXTURE2);
    this.glowTex = this.texture(gl.LINEAR);
    gl.activeTexture(gl.TEXTURE1);
    this.fontTex = this.texture(gl.NEAREST);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.LUMINANCE, 60, 7, 0, gl.LUMINANCE, gl.UNSIGNED_BYTE, fontBytes(this.font));

    this.fbo = gl.createFramebuffer()!;
    this.sizeField(1, 1);

    gl.useProgram(this.glowProg);
    gl.uniform1i(this.ug.uGlow, 2);
    gl.useProgram(this.asciiProg);
    gl.uniform1i(this.ua.uField, 0);
    gl.uniform1i(this.ua.uFont, 1);
    gl.disable(gl.DEPTH_TEST);
    gl.clearColor(0, 0, 0, 1);
  }

  /** (Re)allocates the two field pictures at `w` x `h` cells. */
  private sizeField(w: number, h: number) {
    const gl = this.gl;
    for (const [unit, tex] of [[gl.TEXTURE0, this.fieldTex], [gl.TEXTURE2, this.glowTex]] as const) {
      gl.activeTexture(unit);
      gl.bindTexture(gl.TEXTURE_2D, tex);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    }
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.fbo);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.fieldTex, 0);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT1, gl.TEXTURE_2D, this.glowTex, 0);
    gl.drawBuffers([gl.COLOR_ATTACHMENT0, gl.COLOR_ATTACHMENT1]);
    const ok = gl.checkFramebufferStatus(gl.FRAMEBUFFER) === gl.FRAMEBUFFER_COMPLETE;
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    if (!ok) throw new Error("field framebuffer incomplete");
    this.fw = w; this.fh = h;
  }

  private setRamp(loc: WebGLUniformLocation | null, r: Ramp) {
    const a = this.ramps;
    for (let i = 0; i < 4; i++) { a[i * 3] = r[i][0]; a[i * 3 + 1] = r[i][1]; a[i * 3 + 2] = r[i][2]; }
    this.gl.uniform3fv(loc, a);
  }

  draw(f: GpuFrame) {
    if (this.lost) return;
    const gl = this.gl;
    if (this.fw !== f.iw || this.fh !== f.ih) this.sizeField(f.iw, f.ih);

    // 1. The field, one fragment per cell.
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.fbo);
    gl.viewport(0, 0, f.iw, f.ih);
    gl.disable(gl.BLEND);
    gl.useProgram(this.fieldProg);
    const u = this.uf;
    gl.uniform2f(u.uFieldSize, f.iw, f.ih);
    gl.uniform1f(u.uAspect, f.aspect);
    gl.uniform1i(u.uPreset, f.preset);
    gl.uniform1i(u.uPrevPreset, f.prevPreset);
    gl.uniform1f(u.uPresetT, f.presetT);
    gl.uniform4f(u.uMotion, f.dx, f.dy, f.size, f.gain);
    gl.uniform1f(u.uT, f.t);
    gl.uniform1f(u.uSh, f.sh);
    gl.uniform1f(u.uLevel, f.level);
    this.setRamp(u.uCool, f.cool);
    this.setRamp(u.uWarm, f.warm);
    this.setRamp(u.uPCool, f.prevCool);
    this.setRamp(u.uPWarm, f.prevWarm);
    gl.drawArrays(gl.TRIANGLES, 0, 3);

    // 2. The light over the whole window.
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight);
    gl.useProgram(this.glowProg);
    gl.uniform2f(this.ug.uRes, f.W, f.H);
    gl.drawArrays(gl.TRIANGLES, 0, 3);

    // 3. Characters and titlebar dots on top (premultiplied colours).
    if (!f.ascii && !f.dots) return;
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    gl.useProgram(this.asciiProg);
    const a = this.ua;
    gl.uniform2f(a.uRes, f.W, f.H);
    gl.uniform2f(a.uFieldSize, f.iw, f.ih);
    gl.uniform1f(a.uPitch, f.pitch);
    // No characters: a threshold nothing reaches (the dots stay).
    gl.uniform1f(a.uDensity, f.density);
    gl.uniform1f(a.uThreshold, f.ascii ? f.threshold : 1e9);
    gl.uniform1f(a.uLevel, f.level);
    gl.uniform1f(a.uDotLevel, f.dotLevel);
    gl.uniform1f(a.uBlend, f.blend);
    gl.uniform1f(a.uDots, f.dots ? 1 : 0);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }

  /** Frees the GPU memory right away (the canvas goes too). */
  destroy() {
    this.canvas.removeEventListener("webglcontextlost", this.onLost);
    this.canvas.removeEventListener("webglcontextrestored", this.onRestored);
    if (this.lost) return;
    const gl = this.gl;
    try {
      gl.deleteFramebuffer(this.fbo);
      for (const t of [this.fieldTex, this.glowTex, this.fontTex]) gl.deleteTexture(t);
      for (const p of [this.fieldProg, this.glowProg, this.asciiProg]) gl.deleteProgram(p);
      gl.deleteBuffer(this.buf);
      gl.getExtension("WEBGL_lose_context")?.loseContext();
    } catch {
      // Already gone.
    }
  }
}
