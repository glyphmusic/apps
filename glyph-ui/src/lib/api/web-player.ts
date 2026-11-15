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

  constructor(private hooks: EngineHooks) {
    const a = this.audio;
    a.preload = "auto";
    a.crossOrigin = null;
    a.addEventListener("loadedmetadata", () => {
      if (this.mode !== "audio" || this.pendingStart <= 0) return;
      const target = Number.isFinite(a.duration) ? Math.min(this.pendingStart, a.duration) : this.pendingStart;
      this.pendingStart = 0;
      try { a.currentTime = target; } catch { /* ignore */ }
    });
    a.addEventListener("timeupdate", () => this.mode === "audio" && hooks.time(a.currentTime, Number.isFinite(a.duration) ? a.duration : null));
    a.addEventListener("durationchange", () => this.mode === "audio" && hooks.time(a.currentTime, Number.isFinite(a.duration) ? a.duration : null));
    a.addEventListener("playing", () => this.mode === "audio" && hooks.playing(true));
    a.addEventListener("pause", () => this.mode === "audio" && !a.ended && hooks.playing(false));
    a.addEventListener("waiting", () => this.mode === "audio" && hooks.waiting());
    a.addEventListener("ended", () => this.mode === "audio" && hooks.ended());
    a.addEventListener("error", () => {
      if (this.mode !== "audio" || !a.src) return;
      hooks.error(a.error?.code === MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED ? "format_unsupported" : "playback_failed");
    });
  }

  /** Play (or only load, `autoplay` false) a resolved source from `start` seconds. */
  async load(src: PlaybackSource, autoplay: boolean, start = 0) {
    const gen = ++this.gen;
    if (src.kind === "embed") {
      if (src.provider !== "youtube") throw new Error("embed_unsupported");
      this.stopAudio();
      this.mode = "youtube";
      const yt = await this.youtube();
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
    const a = this.audio;
    this.pendingStart = start > 0 ? start : 0;
    a.src = src.url;
    if (start > 0) {
      try { a.currentTime = start; } catch { /* applied on loadedmetadata */ }
    }
    this.applyVolume();
    if (autoplay) await a.play().catch((e: unknown) => { if (gen === this.gen) this.autoplayBlocked(e); });
  }

  play() {
    if (this.mode === "audio") this.audio.play().catch((e: unknown) => this.autoplayBlocked(e));
    else if (this.mode === "youtube") this.yt?.playVideo();
  }
  pause() {
    if (this.mode === "audio") this.audio.pause();
    else if (this.mode === "youtube") this.yt?.pauseVideo();
  }
  seek(s: number) {
    this.pendingStart = 0;
    if (this.mode === "audio") this.audio.currentTime = s;
    else if (this.mode === "youtube") this.yt?.seekTo(s, true);
  }
  setVolume(v: number, muted: boolean) {
    this.volume = Math.max(0, Math.min(1, v));
    this.muted = muted;
    this.applyVolume();
  }
  stop() {
    this.gen++;
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
    this.pendingStart = 0;
    if (!a.src) return;
    a.pause();
    a.removeAttribute("src");
    a.load();
  }

  private stopYouTube() {
    clearInterval(this.poll);
    if (this.yt) { try { this.yt.stopVideo(); } catch { /* player not ready */ } }
    this.showBox(false);
  }

  /** The visible box that holds YouTube's player (at least 200 px high, as YouTube asks). */
  private showBox(on: boolean) {
    if (!this.box) {
      if (!on) return;
      const b = document.createElement("div");
      b.className = "glyph-yt";
      b.setAttribute("aria-label", "YouTube");
      Object.assign(b.style, {
        position: "fixed", right: "16px", bottom: "112px", zIndex: "25", width: "min(356px, calc(100vw - 32px))", height: "200px",
        borderRadius: "12px", overflow: "hidden", background: "#000", transition: "opacity .2s ease-out",
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
        playerVars: { playsinline: 1, rel: 0, controls: 1, origin: location.origin },
        events: {
          onReady: () => { this.yt = p; this.applyVolume(); resolve(p); },
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
        report();
        this.poll = window.setInterval(report, 250);
        this.hooks.playing(true);
        break;
      case 2: report(); this.hooks.playing(false); break; // paused
      case 3: this.hooks.waiting(); break; // buffering
      case 0: this.hooks.ended(); break; // ended
    }
  }
}
