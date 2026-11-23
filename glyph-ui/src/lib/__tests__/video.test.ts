import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { DemoBackend } from "../api/demo";
import type { PlayerState, VideoResolveResponse, VideoSearchResponse } from "../api/types";
import { VideoState, type VideoAudioBridge } from "../pages/nowplaying/video-state.svelte";

function createAudioHarness(initial: PlayerState["status"] = "playing") {
  let status: PlayerState["status"] = initial;
  const backendListeners = new Set<(s: PlayerState["status"]) => void>();
  const explicitListeners = new Set<() => void>();
  const pause = vi.fn(() => {
    status = "paused";
  });
  const resume = vi.fn(() => {
    status = "playing";
  });
  const bridge: VideoAudioBridge = {
    get playing() {
      return status === "playing";
    },
    get loading() {
      return status === "loading";
    },
    pause,
    resume,
    onBackendPlayerStatus(fn) {
      backendListeners.add(fn);
      return () => backendListeners.delete(fn);
    },
    onExplicitAudioPlay(fn) {
      explicitListeners.add(fn);
      return () => explicitListeners.delete(fn);
    },
  };
  return {
    bridge,
    pause,
    resume,
    get status() {
      return status;
    },
    setStatus(next: PlayerState["status"]) {
      status = next;
    },
    emitBackend(next: PlayerState["status"]) {
      status = next;
      for (const fn of backendListeners) fn(next);
    },
    emitExplicitPlay() {
      for (const fn of explicitListeners) fn();
    },
  };
}

describe("music video demo backend", () => {
  const prevWindow = (globalThis as { window?: unknown }).window;
  beforeAll(() => {
    if (typeof (globalThis as { window?: unknown }).window === "undefined") {
      (globalThis as { window?: unknown }).window = globalThis;
    }
  });
  afterAll(() => {
    if (prevWindow === undefined) delete (globalThis as { window?: unknown }).window;
  });

  it("searches videos for a song with unique IDs, supports custom queries, and resolves playable qualities", async () => {
    const api = new DemoBackend();
    const snap = await api.snapshot();
    const track = snap.player.current!;
    expect(track).toBeTruthy();

    const search = await api.call<VideoSearchResponse>("video_search", { track });
    expect(search.results.length).toBeGreaterThanOrEqual(4);
    expect(new Set(search.results.map((v) => v.id)).size).toBe(search.results.length);
    expect(search.results[0].kind).toBe("official");
    expect(search.results.some((v) => v.kind === "live")).toBe(true);
    expect(search.results.some((v) => v.kind === "lyric")).toBe(true);

    const custom = await api.call<VideoSearchResponse>("video_search", { track, query: "live acoustic" });
    expect(new Set(custom.results.map((v) => v.id)).size).toBe(custom.results.length);
    expect(custom.results[0].title.toLowerCase()).toContain("live acoustic");

    const resolved = await api.call<VideoResolveResponse>("video_resolve", { id: search.results[0].id });
    expect(resolved.id).toBe(search.results[0].id);
    expect(resolved.url).toContain("demo://video/");
    expect(resolved.qualities?.map((q) => q.label)).toEqual(["1080p", "720p", "360p"]);
  });

  it("keeps pause and resume idempotent on repeated calls", async () => {
    const api = new DemoBackend();
    const statuses: string[] = [];
    api.on("player", (p) => {
      if (p.status) statuses.push(p.status);
    });

    await api.call("resume");
    await api.call("resume");
    await api.call("pause");
    await api.call("pause");
    expect(statuses.at(-1)).toBe("paused");
    expect(statuses.filter((s) => s === "paused")).toHaveLength(1);

    await api.call("resume");
    await api.call("resume");
    expect(statuses.at(-1)).toBe("playing");
  });
});

describe("VideoState audio-video handoff", () => {
  it("keeps audio paused across repeated notifyVideoPlaying calls and stale playing ticks", () => {
    let clock = 1000;
    const audio = createAudioHarness("playing");
    const video = new VideoState(audio.bridge, () => clock);
    const pauseVideo = vi.fn();
    const toggleVideo = vi.fn();
    video.registerController({ pause: pauseVideo, togglePlay: toggleVideo });

    // Call #1 from <video onplaying>
    video.notifyVideoPlaying();
    expect(audio.pause).toHaveBeenCalledTimes(1);
    expect(audio.resume).not.toHaveBeenCalled();
    expect(video.pausedAudioForVideo).toBe(true);

    // Stale backend tick arrives with "playing" before the audio thread processed pause
    clock += 10;
    audio.emitBackend("playing");
    expect(pauseVideo).not.toHaveBeenCalled();
    expect(audio.pause).toHaveBeenCalledTimes(2);
    expect(audio.resume).not.toHaveBeenCalled();

    // Call #2 from startPlayback() after awaiting audioEl.play()
    clock += 15;
    video.notifyVideoPlaying();
    expect(audio.resume).not.toHaveBeenCalled();
    expect(pauseVideo).not.toHaveBeenCalled();

    // Backend confirms paused
    clock += 50;
    audio.emitBackend("paused");
    expect(video.playing).toBe(true);
    expect(audio.status).toBe("paused");

    // Leaving video mode resumes audio exactly once even if leave() is called twice
    video.leave(true);
    video.leave(true);
    expect(audio.resume).toHaveBeenCalledTimes(1);
  });

  it("pauses audio that is still loading when video playback starts and blocks late bleed-through", () => {
    let clock = 2000;
    const audio = createAudioHarness("loading");
    const video = new VideoState(audio.bridge, () => clock);
    const pauseVideo = vi.fn();
    video.registerController({ pause: pauseVideo, togglePlay: vi.fn() });

    video.notifyVideoPlaying();
    expect(audio.pause).toHaveBeenCalledTimes(1);
    expect(video.pausedAudioForVideo).toBe(true);

    // Backend continues reporting "loading" while resolving: must not loop or pause video
    audio.emitBackend("loading");
    expect(audio.pause).toHaveBeenCalledTimes(1);
    expect(pauseVideo).not.toHaveBeenCalled();

    // Even if an in-flight load briefly reports "playing", video re-enforces pause
    clock += 100;
    audio.emitBackend("playing");
    expect(audio.pause).toHaveBeenCalledTimes(2);
    expect(pauseVideo).not.toHaveBeenCalled();
  });

  it("pauses video when user explicitly plays audio or audio resumes externally after settling", () => {
    let clock = 5000;
    const audio = createAudioHarness("playing");
    const video = new VideoState(audio.bridge, () => clock);
    const pauseVideo = vi.fn(() => video.notifyVideoPaused());
    video.registerController({ pause: pauseVideo, togglePlay: vi.fn() });

    video.notifyVideoPlaying();
    audio.emitBackend("paused");

    // Explicit Play click in PlayerBar pauses the video so audio never overlays
    audio.emitExplicitPlay();
    expect(pauseVideo).toHaveBeenCalledTimes(1);
    expect(video.playing).toBe(false);
    expect(video.pausedAudioForVideo).toBe(false);

    // Restart video, settle audio paused, then advance past grace window and simulate media key Play
    clock += 500;
    audio.setStatus("paused");
    video.notifyVideoPlaying();
    audio.emitBackend("paused");
    clock += 2000;
    audio.emitBackend("playing");
    expect(pauseVideo).toHaveBeenCalledTimes(2);
    expect(video.playing).toBe(false);
  });

  it("routes play/pause shortcut to the mounted video player only when a stream is active", () => {
    const audio = createAudioHarness("paused");
    const video = new VideoState(audio.bridge);
    const toggleVideo = vi.fn();
    const unregister = video.registerController({ pause: vi.fn(), togglePlay: toggleVideo });

    expect(video.handlePlayPauseShortcut()).toBe(false);

    video.stream = { id: "vid1", url: "https://example.invalid/v.mp4" };
    expect(video.handlePlayPauseShortcut()).toBe(true);
    expect(toggleVideo).toHaveBeenCalledTimes(1);

    unregister();
    expect(video.handlePlayPauseShortcut()).toBe(false);
  });
});
