// Sound for the web player. Streams and previews play in an <audio> element
// straight from the provider's URL; YouTube plays only in YouTube's official
// IFrame player (privacy host youtube-nocookie.com), which stays visible in a
// small box while it plays, as YouTube asks. No audio ever goes through
// glyph-server.
import type { PlaybackSource } from "./web-server";

export interface EngineHooks {
  /** Seconds. */
  time(position: number, duration: number | null): void;
  playing(on: boolean): void;
  waiting(): void;
  ended(): void;
  error(message: string): void;
}

/** Minimal types of the YouTube IFrame API we use. */
interface YTPlayer {
  loadVideoById(o: { videoId: string; startSeconds?: number }): void;
  cueVideoById(o: { videoId: string; startSeconds?: number }): void;
  playVideo(): void;
  pauseVideo(): void;
  stopVideo(): void;
  seekTo(s: number, allowSeekAhead: boolean): void;
  setVolume(v: number): void;
  mute(): void;
  unMute(): void;
  getCurrentTime(): number;
  getDuration(): number;
  destroy(): void;
}
interface YTNamespace {
  Player: new (el: HTMLElement, o: Record<string, unknown>) => YTPlayer;
}
declare global {
  interface Window { YT?: YTNamespace; onYouTubeIframeAPIReady?: () => void }
}

const YT_API = "https://www.youtube.com/iframe_api";

/** Builds a 1-second silent 16-bit mono PCM WAV Blob URL (allowed by `media-src blob:`). */
function createSilentWavUrl(): string | null {
  if (typeof URL === "undefined" || typeof Blob === "undefined" || typeof URL.createObjectURL !== "function") {
    return null;
  }
  const sampleRate = 8000;
  const numSamples = sampleRate;
  const dataBytes = numSamples * 2;
  const buf = new ArrayBuffer(44 + dataBytes);
  const view = new DataView(buf);
  const writeStr = (off: number, s: string) => {
    for (let i = 0; i < s.length; i++) view.setUint8(off + i, s.charCodeAt(i));
  };
  writeStr(0, "RIFF");
  view.setUint32(4, 36 + dataBytes, true);
  writeStr(8, "WAVE");
  writeStr(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  writeStr(36, "data");
  view.setUint32(40, dataBytes, true);
  return URL.createObjectURL(new Blob([buf], { type: "audio/wav" }));
}

/** A URL that would stream through glyph-server (same origin, `/v1/audio/...`, a short link's audio). */
export function serverAudio(url: string): boolean {
  try {
    const u = new URL(url, location.href);
    if (u.protocol !== "https:" && u.protocol !== "http:") return true;
    return u.origin === location.origin || /^\/v1\/(audio|download)\//.test(u.pathname) || /^\/p\/[^/]+\/audio\//.test(u.pathname);
  } catch {
    return true;
  }
}

let ytApi: Promise<YTNamespace> | null = null;
function loadYouTubeApi(): Promise<YTNamespace> {
  if (window.YT?.Player) return Promise.resolve(window.YT);
  if (ytApi) return ytApi;
  ytApi = new Promise((resolve, reject) => {
    const prev = window.onYouTubeIframeAPIReady;
    window.onYouTubeIframeAPIReady = () => { prev?.(); if (window.YT) resolve(window.YT); };
    const s = document.createElement("script");
    s.src = YT_API;
    s.async = true;
    s.onerror = () => { ytApi = null; reject(new Error("youtube_unavailable")); };
    document.head.appendChild(s);
  });
  return ytApi;
}

export class WebEngine {
  private audio = new Audio();
  private nextAudio = new Audio();
  private preloadedUrl: string | null = null;
  private mode: "audio" | "youtube" | null = null;
  private yt: YTPlayer | null = null;
  private ytReady: Promise<YTPlayer> | null = null;
  private box: HTMLDivElement | null = null;
  private poll = 0;
  private volume = 0.8;
  private muted = false;
  /** Bumped on every load, so late events of an old song are dropped. */
  private gen = 0;
  /** Initial seek offset applied once the `<audio>` element has metadata. */
  private pendingStart = 0;
  /** True once `this.audio` has been started by a play request. */
  private audioUnlocked = false;
  /** Silent blob WAV keepalive active while resolving the next track asynchronously. */
  private keepalive = false;
  /** Suppresses transient `<audio>` pause events fired while swapping `src` during load. */
  private loadingAudio = false;
  private silentUrl: string | null = null;
  private ytWantedPlaying = false;
  private ytAutoResumeUsed = false;
  private ytResumeTimer = 0;
  private hiddenAt = 0;

  constructor(private hooks: EngineHooks) {
    this.bindAudio(this.audio);
    this.nextAudio.preload = "auto";
    this.nextAudio.muted = true;
    this.nextAudio.crossOrigin = null;
    this.nextAudio.addEventListener("error", () => {
      if (this.nextAudio.src && this.preloadedUrl === this.nextAudio.src) {
        this.preloadedUrl = null;
      }
    });
    if (typeof document !== "undefined") {
      document.addEventListener("visibilitychange", () => {
        if (document.visibilityState === "hidden") {
          this.hiddenAt = performance.now();
          this.ytAutoResumeUsed = false;
        } else {
          this.ytAutoResumeUsed = false;
          clearTimeout(this.ytResumeTimer);
        }
      });
    }
  }

  private bindAudio(a: HTMLAudioElement) {
    a.preload = "auto";
    a.crossOrigin = null;
    a.setAttribute("playsinline", "");
    a.addEventListener("loadedmetadata", () => {
      if (this.keepalive || this.mode !== "audio" || this.pendingStart <= 0) return;
      const target = Number.isFinite(a.duration) ? Math.min(this.pendingStart, a.duration) : this.pendingStart;
      this.pendingStart = 0;
      try { a.currentTime = target; } catch { /* ignore */ }
    });
    a.addEventListener("timeupdate", () => !this.keepalive && this.mode === "audio" && this.hooks.time(a.currentTime, Number.isFinite(a.duration) ? a.duration : null));
    a.addEventListener("durationchange", () => !this.keepalive && this.mode === "audio" && this.hooks.time(a.currentTime, Number.isFinite(a.duration) ? a.duration : null));
    a.addEventListener("playing", () => {
      this.loadingAudio = false;
      if (!this.keepalive && this.mode === "audio") this.hooks.playing(true);
    });
    a.addEventListener("pause", () => !this.keepalive && !this.loadingAudio && this.mode === "audio" && !a.ended && this.hooks.playing(false));
    a.addEventListener("waiting", () => !this.keepalive && this.mode === "audio" && this.hooks.waiting());
    a.addEventListener("ended", () => {
      this.loadingAudio = false;
      if (!this.keepalive && this.mode === "audio") this.hooks.ended();
    });
    a.addEventListener("error", () => {
      this.loadingAudio = false;
      if (this.keepalive || this.mode !== "audio" || !a.src) return;
      this.hooks.error(a.error?.code === MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED ? "format_unsupported" : "playback_failed");
    });
  }

  private getSilentWavUrl(): string | null {
    if (!this.silentUrl) this.silentUrl = createSilentWavUrl();
    return this.silentUrl;
  }

  /** Pre-buffer the next direct audio stream (or warm the YouTube API) without interrupting playback. */
  preload(src: PlaybackSource | null) {
    if (!src) return;
    if (src.kind === "embed") {
      if (src.provider === "youtube") void loadYouTubeApi().catch(() => {});
      return;
    }
    if (serverAudio(src.url) || this.preloadedUrl === src.url) return;
    this.preloadedUrl = src.url;
    this.nextAudio.src = src.url;
    this.nextAudio.load();
  }

  /** Keep (or synchronously unlock on iOS Safari) the `<audio>` session during async track resolution. */
  beginTransition() {
    this.gen++;
    clearTimeout(this.ytResumeTimer);
    this.stopYouTube();
    const url = this.getSilentWavUrl();
    if (!url) {
      if (this.mode === "audio") {
        if (!this.audio.paused) this.audio.pause();
      } else {
        this.stopAudio();
        this.mode = null;
      }
      return;
    }
    this.mode = "audio";
    this.pendingStart = 0;
    this.keepalive = true;
    const a = this.audio;
    a.loop = true;
    if (a.src !== url) a.src = url;
    this.audioUnlocked = true;
    void a.play().catch(() => {});
  }

  /** Play (or only load, `autoplay` false) a resolved source from `start` seconds. */
  async load(src: PlaybackSource, autoplay: boolean, start = 0) {
    const gen = ++this.gen;
    clearTimeout(this.ytResumeTimer);
    if (src.kind === "embed") {
      if (src.provider !== "youtube") throw new Error("embed_unsupported");
      this.stopAudio();
      this.mode = "youtube";
      this.ytWantedPlaying = autoplay;
      this.ytAutoResumeUsed = false;
      const yt = this.yt ?? await this.youtube();
      if (gen !== this.gen) return;
      this.showBox(true);
      const o = { videoId: src.embed_id, startSeconds: start || undefined };
      if (autoplay) yt.loadVideoById(o); else yt.cueVideoById(o);
      this.applyVolume();
      return;
    }
    // A server audio path: the web player never plays audio through the server.
    if (serverAudio(src.url)) throw new Error("server_audio");
    this.stopYouTube();
    this.mode = "audio";
    this.keepalive = false;
    this.loadingAudio = true;
    if (this.preloadedUrl === src.url) {
      this.preloadedUrl = null;
      this.nextAudio.removeAttribute("src");
      this.nextAudio.load();
    }
    const a = this.audio;
    a.loop = false;
    this.pendingStart = start > 0 ? start : 0;
    if (a.src !== src.url) {
      a.src = src.url;
    }
    if (start > 0) {
      try { a.currentTime = start; } catch { /* applied on loadedmetadata */ }
    }
    this.applyVolume();
    if (autoplay) {
      this.audioUnlocked = true;
      await a.play().catch((e: unknown) => {
        this.loadingAudio = false;
        if (gen === this.gen) this.autoplayBlocked(e);
      });
    } else {
      this.loadingAudio = false;
    }
  }

  play() {
    if (this.mode === "audio") {
      this.audioUnlocked = true;
      this.audio.play().catch((e: unknown) => this.autoplayBlocked(e));
    } else if (this.mode === "youtube") {
      this.ytWantedPlaying = true;
      this.yt?.playVideo();
    }
  }
  pause() {
    clearTimeout(this.ytResumeTimer);
    if (this.mode === "audio") {
      this.keepalive = false;
      this.audio.loop = false;
      this.audio.pause();
    } else if (this.mode === "youtube") {
      this.ytWantedPlaying = false;
      this.yt?.pauseVideo();
    }
  }
  seek(s: number) {
    this.pendingStart = 0;
    if (this.mode === "audio" && !this.keepalive) this.audio.currentTime = s;
    else if (this.mode === "youtube") this.yt?.seekTo(s, true);
  }
  setVolume(v: number, muted: boolean) {
    this.volume = Math.max(0, Math.min(1, v));
    this.muted = muted;
    this.applyVolume();
  }
  stop() {
    this.gen++;
    clearTimeout(this.ytResumeTimer);
    this.stopAudio();
    this.stopYouTube();
    this.mode = null;
  }

  private applyVolume() {
    this.audio.volume = this.volume;
    this.audio.muted = this.muted;
    if (this.yt) {
      this.yt.setVolume(Math.round(this.volume * 100));
      if (this.muted) this.yt.mute(); else this.yt.unMute();
    }
  }

  private autoplayBlocked(e: unknown) {
    // Browsers block sound until the visitor clicks once: show "paused".
    if ((e as DOMException)?.name === "NotAllowedError") this.hooks.playing(false);
    else if ((e as DOMException)?.name !== "AbortError") this.hooks.error("playback_failed");
  }

  private stopAudio() {
    const a = this.audio;
    this.keepalive = false;
    this.loadingAudio = false;
    this.pendingStart = 0;
    a.loop = false;
    if (!a.src) return;
    a.pause();
    a.removeAttribute("src");
    a.load();
  }

  private stopYouTube() {
    clearInterval(this.poll);
    clearTimeout(this.ytResumeTimer);
    this.ytWantedPlaying = false;
    this.ytAutoResumeUsed = false;
    if (this.yt) { try { this.yt.stopVideo(); } catch { /* player not ready */ } }
    this.showBox(false);
  }

  /** Hidden audio-only YouTube IFrame host (kept laid out for playback without showing a video box). */
  private showBox(on: boolean) {
    if (!this.box) {
      if (!on) return;
      const b = document.createElement("div");
      b.className = "glyph-yt";
      b.setAttribute("aria-hidden", "true");
      Object.assign(b.style, {
        position: "fixed", left: "-240px", bottom: "0", zIndex: "-9999", width: "200px", height: "200px",
        opacity: "0", clipPath: "inset(50%)", pointerEvents: "none", overflow: "hidden",
      } satisfies Partial<CSSStyleDeclaration>);
      const inner = document.createElement("div");
      b.appendChild(inner);
      document.body.appendChild(b);
      this.box = b;
    }
    this.box.style.display = on ? "block" : "none";
  }

  private youtube(): Promise<YTPlayer> {
    if (this.ytReady) return this.ytReady;
    this.showBox(true);
    const el = this.box!.firstElementChild as HTMLElement;
    this.ytReady = loadYouTubeApi().then((YT) => new Promise<YTPlayer>((resolve) => {
      const p = new YT.Player(el, {
        host: "https://www.youtube-nocookie.com",
        width: "100%", height: "100%",
        playerVars: { playsinline: 1, rel: 0, controls: 0, origin: location.origin },
        events: {
          onReady: () => {
            this.box?.querySelector("iframe")?.setAttribute("tabindex", "-1");
            this.yt = p; this.applyVolume(); resolve(p);
          },
          onStateChange: (e: { data: number }) => this.ytState(e.data),
          onError: (e: { data: number }) => this.mode === "youtube" && this.hooks.error(e.data === 101 || e.data === 150 ? "embed_blocked" : "playback_failed"),
        },
      });
    }));
    this.ytReady.catch(() => { this.ytReady = null; });
    return this.ytReady;
  }

  private ytState(s: number) {
    if (this.mode !== "youtube" || !this.yt) return;
    const yt = this.yt;
    const report = () => { const d = yt.getDuration(); this.hooks.time(yt.getCurrentTime(), d > 0 ? d : null); };
    clearInterval(this.poll);
    switch (s) {
      case 1: // playing
        this.ytAutoResumeUsed = false;
        report();
        this.poll = window.setInterval(report, 250);
        this.hooks.playing(true);
        break;
      case 2: // paused
        if (
          this.ytWantedPlaying
          && typeof document !== "undefined"
          && document.hidden
          && !this.ytAutoResumeUsed
          && performance.now() - this.hiddenAt < 1500
        ) {
          this.ytAutoResumeUsed = true;
          clearTimeout(this.ytResumeTimer);
          this.ytResumeTimer = window.setTimeout(() => {
            if (this.mode === "youtube" && this.ytWantedPlaying && this.yt) {
              try { this.yt.playVideo(); } catch { /* ignore */ }
            }
          }, 120);
          return;
        }
        this.ytWantedPlaying = false;
        report();
        this.hooks.playing(false);
        break;
      case 3: this.hooks.waiting(); break; // buffering
      case 5: // cued (can happen in background tab after loadVideoById)
        if (this.ytWantedPlaying) {
          try { yt.playVideo(); } catch { /* ignore */ }
        }
        break;
      case 0: this.hooks.ended(); break; // ended
    }
  }
}
