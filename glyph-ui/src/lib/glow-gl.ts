// The ASCII layer and titlebar dots of the background (glow.ts) drawn by
// the GPU: the CPU only computes the small glow field (one sample per
// 8 x 8 px), uploads it as a tiny texture, and a fragment shader paints the
// characters at full size. Before, the CPU filled and uploaded a full-window
// picture (about 8 MB at 1080p) 6-12 times a second.
// Same cells (10 x 16), font (5 x 7), ramp, thresholds and colours as the
// CPU path in glow.ts, which stays as the fallback without WebGL.

const VERT = `
attribute vec2 aPos;
void main() { gl_Position = vec4(aPos, 0.0, 1.0); }
`;

const FRAG = `
precision highp float;
uniform sampler2D uField;
uniform sampler2D uFont;
uniform vec2 uRes;
uniform vec2 uFieldSize;
uniform float uPitch;
uniform float uDensity;
uniform float uThreshold;
uniform float uLevel;
uniform float uDotLevel;
uniform float uBlend;
uniform float uDots;

vec4 fieldAt(vec2 p) {
  vec2 cell = min(floor(p / uPitch), uFieldSize - 1.0);
  return texture2D(uField, (cell + 0.5) / uFieldSize);
}

void main() {
  float x = floor(gl_FragCoord.x);
  float y = floor(uRes.y - gl_FragCoord.y);
  if (uDots > 0.5 && y < 44.0) {
    float mx = mod(x, 4.0), my = mod(y, 4.0);
    if (mx < 1.0 || mx >= 3.0 || my < 1.0 || my >= 3.0) discard;
    float lx = floor(x / 4.0) * 4.0 + 1.0;
    float k = floor(y / 4.0);
    vec4 f = fieldAt(vec2(lx, 6.0));
    float b = f.a * 2.0;
    float mt = max(smoothstep(0.0, 0.6, b), uBlend * 0.55);
    vec3 c = mix(vec3(12.0, 26.0, 107.0) / 255.0, f.rgb, mt);
    float al = min(1.0, uDotLevel * (1.0 - (k / 11.0) * 0.35));
    gl_FragColor = vec4(c * al, al);
    return;
  }
  float col = floor(x / 10.0), row = floor(y / 16.0);
  float lx = col * 10.0, ly = row * 16.0;
  if (col >= floor(uRes.x / 10.0) || ly + 16.0 > uRes.y || (uDots > 0.5 && ly < 44.0)) discard;
  float gx = x - lx - 2.0, gy = y - ly - 4.0;
  if (gx < 0.0 || gx >= 5.0 || gy < 0.0 || gy >= 7.0) discard;
  vec4 f = fieldAt(vec2(lx + 5.0, ly + 8.0));
  float dv = f.a * 2.0 * uDensity;
  if (dv < uThreshold) discard;
  float idx = clamp(floor(min(dv, 1.0) * 11.0 + 0.5), 1.0, 11.0);
  float bit = texture2D(uFont, vec2((idx * 5.0 + gx + 0.5) / 60.0, (gy + 0.5) / 7.0)).r;
  if (bit < 0.5) discard;
  vec3 c = f.rgb + (1.0 - f.rgb) * 0.3;
  float al = (0.16 + 0.16 * min(dv, 1.0)) * uLevel;
  gl_FragColor = vec4(c * al, al);
}
`;

export interface AsciiFrame {
  W: number; H: number; iw: number; ih: number; pitch: number;
  /** RGB per field cell, 0..1. */
  cols: Float32Array;
  /** Brightness per field cell (can exceed 1). */
  bright: Float32Array;
  density: number; threshold: number; level: number; dotLevel: number; blend: number; dots: boolean;
}

export class AsciiGl {
  private gl: WebGLRenderingContext;
  private fieldTex: WebGLTexture;
  private fieldBytes = new Uint8Array(0);
  private fw = 0; private fh = 0;
  private u: Record<string, WebGLUniformLocation | null> = {};
  /** The GPU dropped the context (driver reset, too many contexts): use the CPU path. */
  lost = false;

  /** null when the canvas has no WebGL (old WebView, blocked GPU). */
  static create(canvas: HTMLCanvasElement, font: number[][]): AsciiGl | null {
    try {
      const opts: WebGLContextAttributes = { premultipliedAlpha: true, alpha: true, antialias: false, depth: false, stencil: false, preserveDrawingBuffer: false, powerPreference: "low-power" };
      const gl = (canvas.getContext("webgl", opts) ?? canvas.getContext("experimental-webgl", opts)) as WebGLRenderingContext | null;
      if (!gl) return null;
      return new AsciiGl(canvas, gl, font);
    } catch (e) {
      console.warn("[glow] WebGL ASCII layer unavailable, using the CPU:", e);
      return null;
    }
  }

  private constructor(canvas: HTMLCanvasElement, gl: WebGLRenderingContext, font: number[][]) {
    this.gl = gl;
    canvas.addEventListener("webglcontextlost", (e) => { e.preventDefault(); this.lost = true; });
    const sh = (type: number, src: string) => {
      const s = gl.createShader(type)!;
      gl.shaderSource(s, src);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s) ?? "shader");
      return s;
    };
    const prog = gl.createProgram()!;
    gl.attachShader(prog, sh(gl.VERTEX_SHADER, VERT));
    gl.attachShader(prog, sh(gl.FRAGMENT_SHADER, FRAG));
    gl.linkProgram(prog);
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(prog) ?? "link");
    gl.useProgram(prog);
    const buf = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buf);
    // One triangle that covers the whole canvas.
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
    const loc = gl.getAttribLocation(prog, "aPos");
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    for (const n of ["uField", "uFont", "uRes", "uFieldSize", "uPitch", "uDensity", "uThreshold", "uLevel", "uDotLevel", "uBlend", "uDots"]) {
      this.u[n] = gl.getUniformLocation(prog, n);
    }
    // Font: the 12 ramp glyphs side by side, 5 x 7 bits each.
    const fontBytes = new Uint8Array(60 * 7);
    font.forEach((g, gi) => g.forEach((bits, ry) => {
      for (let rx = 0; rx < 5; rx++) if (bits & (16 >> rx)) fontBytes[ry * 60 + gi * 5 + rx] = 255;
    }));
    const tex = (unit: number) => {
      const t = gl.createTexture()!;
      gl.activeTexture(gl.TEXTURE0 + unit);
      gl.bindTexture(gl.TEXTURE_2D, t);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      return t;
    };
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    tex(1);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.LUMINANCE, 60, 7, 0, gl.LUMINANCE, gl.UNSIGNED_BYTE, fontBytes);
    this.fieldTex = tex(0);
    gl.uniform1i(this.u.uField, 0);
    gl.uniform1i(this.u.uFont, 1);
    gl.disable(gl.BLEND);
    gl.clearColor(0, 0, 0, 0);
  }

  clear() {
    if (this.lost) return;
    const gl = this.gl;
    gl.viewport(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight);
    gl.clear(gl.COLOR_BUFFER_BIT);
  }

  draw(f: AsciiFrame) {
    if (this.lost) return;
    const gl = this.gl;
    const n = f.iw * f.ih;
    if (this.fieldBytes.length !== n * 4) this.fieldBytes = new Uint8Array(n * 4);
    const d = this.fieldBytes, c = f.cols, b = f.bright;
    for (let i = 0; i < n; i++) {
      const o = i * 4, ci = i * 3;
      d[o] = c[ci] * 255; d[o + 1] = c[ci + 1] * 255; d[o + 2] = c[ci + 2] * 255;
      // Brightness / 2 (0..2 fits a byte; above 2 every glyph is the densest anyway).
      const v = b[i] * 127.5;
      d[o + 3] = v > 255 ? 255 : v;
    }
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.fieldTex);
    if (this.fw !== f.iw || this.fh !== f.ih) {
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, f.iw, f.ih, 0, gl.RGBA, gl.UNSIGNED_BYTE, d);
      this.fw = f.iw; this.fh = f.ih;
    } else {
      gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, f.iw, f.ih, gl.RGBA, gl.UNSIGNED_BYTE, d);
    }
    gl.viewport(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight);
    gl.uniform2f(this.u.uRes, f.W, f.H);
    gl.uniform2f(this.u.uFieldSize, f.iw, f.ih);
    gl.uniform1f(this.u.uPitch, f.pitch);
    gl.uniform1f(this.u.uDensity, f.density);
    gl.uniform1f(this.u.uThreshold, f.threshold);
    gl.uniform1f(this.u.uLevel, f.level);
    gl.uniform1f(this.u.uDotLevel, f.dotLevel);
    gl.uniform1f(this.u.uBlend, f.blend);
    gl.uniform1f(this.u.uDots, f.dots ? 1 : 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }
}
