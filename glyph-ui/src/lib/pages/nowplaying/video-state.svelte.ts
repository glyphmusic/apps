// Shared music-video state for Desktop Now Playing and Mobile NowPlayingSheet.
// Handles video search, stream resolution, per-song remembered picks, and clean
// handoff between Glyph's native audio player and the video player.
import { backend } from "../../api/backend";
import type {
  PlayerState,
  Track,
  VideoCandidate,
  VideoKind,
  VideoQuality,
  VideoResolveResponse,
  VideoSearchResponse,
} from "../../api/types";
import { trackKey } from "../../api/types";
import { app } from "../../state.svelte";

export type VideoFilter = "all" | "official" | "live" | "lyric";

export interface VideoAudioBridge {
  readonly playing: boolean;
  readonly loading: boolean;
  pause(): unknown;
  resume(): unknown;
  onBackendPlayerStatus(fn: (status: PlayerState["status"]) => void): () => void;
  onExplicitAudioPlay(fn: () => void): () => void;
}

export interface VideoPlaybackController {
  pause(): void;
  togglePlay(): void;
}

const PAUSE_GRACE_MS = 1500;
const PAUSE_DEDUPE_MS = 150;
const MAX_PAUSE_RETRIES = 3;

const PICKS_KEY = "glyph.video.picks.v1";

function loadPicks(): Record<string, string> {
  try {
    const raw = localStorage.getItem(PICKS_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

function savePicks(picks: Record<string, string>) {
  try {
    const entries = Object.entries(picks).slice(-200);
    localStorage.setItem(PICKS_KEY, JSON.stringify(Object.fromEntries(entries)));
  } catch {
    /* ignore storage quota errors */
  }
}

export class VideoState {
  forTrackKey = $state<string | null>(null);
  query = $state("");
  filter = $state<VideoFilter>("all");
  loading = $state(false);
  resolving = $state(false);
  error = $state<string | null>(null);
  results = $state<VideoCandidate[]>([]);
  selected = $state<VideoCandidate | null>(null);
  stream = $state<VideoResolveResponse | null>(null);
  /** `"auto"` or a specific quality label like `"720p"`, `"360p"`. */
  quality = $state<string>("auto");
  playing = $state(false);
  /** Whether we paused Glyph's main audio engine when video playback started. */
  pausedAudioForVideo = $state(false);
  /** Desktop side panel visibility. */
  listOpen = $state(true);

  private picks: Record<string, string> = typeof localStorage !== "undefined" ? loadPicks() : {};
  private searchSeq = 0;
  private resolveSeq = 0;
  private explicitAudio: VideoAudioBridge | null;
  private bound = false;
  private now: () => number;
  private controller: VideoPlaybackController | null = null;
  private awaitingAudioPause = false;
  private pauseGraceUntil = 0;
  private lastPauseSentAt = -Infinity;
  private pauseRetries = 0;

  constructor(
    audio: VideoAudioBridge | null = null,
    now: () => number = () => (typeof performance !== "undefined" ? performance.now() : Date.now()),
  ) {
    this.explicitAudio = audio;
    this.now = now;
    // The shared pp is resolved lazily: this module is part of an import cycle with
    // state.svelte.ts, so pp is not yet initialised while this file is evaluated
    // (reading it here crashed startup with a blank screen).
    if (audio) this.bind();
  }

  private get audio(): VideoAudioBridge {
    return this.explicitAudio ?? app;
  }

  private bind() {
    if (this.bound) return;
    this.bound = true;
    this.audio.onBackendPlayerStatus((status) => this.handleBackendAudioStatus(status));
    this.audio.onExplicitAudioPlay(() => this.handleExplicitAudioPlay());
  }

  registerController(ctrl: VideoPlaybackController): () => void {
    this.bind();
    this.controller = ctrl;
    return () => {
      if (this.controller === ctrl) this.controller = null;
    };
  }

  /** Handles play/pause keyboard shortcut when a video stream is mounted. */
  handlePlayPauseShortcut(): boolean {
    if (!this.controller || !this.stream) return false;
    this.controller.togglePlay();
    return true;
  }

  get filtered(): VideoCandidate[] {
    if (this.filter === "all") return this.results;
    return this.results.filter((v) => v.kind === this.filter);
  }

  get activeQuality(): VideoQuality | null {
    const qs = this.stream?.qualities;
    if (!qs?.length) return null;
    if (this.quality !== "auto") {
      const exact = qs.find((q) => q.label === this.quality);
      if (exact) return exact;
    }
    return qs.find((q) => q.height === 720) ?? qs[0] ?? null;
  }

  private sendAudioPause(force = false) {
    const t = this.now();
    this.awaitingAudioPause = true;
    this.pauseGraceUntil = Math.max(this.pauseGraceUntil, t + PAUSE_GRACE_MS);
    if (!force && t - this.lastPauseSentAt < PAUSE_DEDUPE_MS && !this.audio.playing && !this.audio.loading) {
      return;
    }
    this.lastPauseSentAt = t;
    void this.audio.pause();
  }

  private handleBackendAudioStatus(status: PlayerState["status"]) {
    if (status === "paused" || status === "idle" || status === "error") {
      this.awaitingAudioPause = false;
      this.pauseRetries = 0;
      return;
    }
    if (!this.playing) return;
    if (status === "loading") {
      this.pausedAudioForVideo = true;
      if (!this.awaitingAudioPause) {
        this.pauseRetries = 0;
        this.sendAudioPause(true);
      }
      return;
    }
    // status === "playing" while video is playing:
    if (this.awaitingAudioPause || this.now() < this.pauseGraceUntil) {
      // Stale backend tick or in-flight load from before pause landed: re-enforce pause.
      this.pausedAudioForVideo = true;
      if (this.pauseRetries < MAX_PAUSE_RETRIES) {
        this.pauseRetries++;
        this.sendAudioPause(true);
      }
      return;
    }
    // Audio was already confirmed paused and resumed externally (media keys, Connect, etc.):
    // pause the video so both audio streams never play simultaneously.
    this.pausedAudioForVideo = false;
    this.controller?.pause();
  }

  private handleExplicitAudioPlay() {
    this.awaitingAudioPause = false;
    this.pauseGraceUntil = 0;
    this.pauseRetries = 0;
    this.pausedAudioForVideo = false;
    if (this.playing) {
      this.controller?.pause();
    }
  }

  /** Idempotently pauses Glyph's audio engine whenever the video starts playing. */
  notifyVideoPlaying() {
    this.playing = true;
    if (this.audio.playing || this.audio.loading) {
      this.pausedAudioForVideo = true;
      this.pauseRetries = 0;
      this.sendAudioPause(true);
    } else if (this.awaitingAudioPause) {
      this.sendAudioPause(false);
    }
  }

  notifyVideoPaused() {
    this.playing = false;
  }

  /** Called when leaving video mode back to Song / Cover / Lyrics. */
  leave(resumeAudioIfPaused = true) {
    const wasPlaying = this.playing;
    const shouldResume = resumeAudioIfPaused && this.pausedAudioForVideo && wasPlaying && !this.audio.playing;
    this.playing = false;
    this.awaitingAudioPause = false;
    this.pauseGraceUntil = 0;
    this.pauseRetries = 0;
    this.pausedAudioForVideo = false;
    if (shouldResume) {
      void this.audio.resume();
    }
  }

  async ensureForTrack(track: Track) {
    const key = trackKey(track);
    if (this.forTrackKey === key && (this.results.length > 0 || this.loading)) {
      if (!this.stream && this.selected && !this.resolving) {
        await this.resolve(this.selected);
      }
      return;
    }
    this.forTrackKey = key;
    this.query = "";
    this.filter = "all";
    this.results = [];
    this.selected = null;
    this.stream = null;
    this.error = null;
    await this.search(track, null, true);
  }

  async search(track: Track, customQuery: string | null, autoSelect = false) {
    const seq = ++this.searchSeq;
    const key = trackKey(track);
    this.forTrackKey = key;
    this.loading = true;
    this.error = null;
    try {
      const q = customQuery?.trim() || null;
      const resp = await backend().call<VideoSearchResponse>("video_search", { track, query: q });
      if (seq !== this.searchSeq) return;
      const raw = resp?.results ?? [];
      const seen = new Set<string>();
      this.results = raw.filter((r) => {
        if (!r?.id || seen.has(r.id)) return false;
        seen.add(r.id);
        return true;
      });
      // Release search skeleton immediately so the list renders and <video>
      // mounts cleanly as soon as resolve() completes.
      this.loading = false;
      if (autoSelect || !this.selected) {
        const savedId = !q ? this.picks[key] : undefined;
        const pick =
          (savedId ? this.results.find((r) => r.id === savedId) : null) ??
          this.results[0] ??
          null;
        if (pick) {
          await this.select(pick, track, false);
        }
      }
    } catch (e) {
      if (seq !== this.searchSeq) return;
      this.error = e instanceof Error ? e.message : String(e);
    } finally {
      if (seq === this.searchSeq) this.loading = false;
    }
  }

  async select(candidate: VideoCandidate, track: Track | null, remember = true) {
    this.selected = candidate;
    if (remember && track) {
      this.picks[trackKey(track)] = candidate.id;
      savePicks(this.picks);
    }
    await this.resolve(candidate);
  }

  async resolve(candidate: VideoCandidate) {
    const seq = ++this.resolveSeq;
    this.resolving = true;
    this.error = null;
    this.stream = null;
    try {
      const res = await backend().call<VideoResolveResponse>("video_resolve", { id: candidate.id });
      if (seq !== this.resolveSeq) return;
      if (!res?.url) throw new Error("No playable video stream returned");
      this.stream = res;
    } catch (e) {
      if (seq !== this.resolveSeq) return;
      this.error = e instanceof Error ? e.message : String(e);
    } finally {
      if (seq === this.resolveSeq) this.resolving = false;
    }
  }

  kindLabelKey(kind: VideoKind): string {
    switch (kind) {
      case "official": return "video.kind.official";
      case "live": return "video.kind.live";
      case "lyric": return "video.kind.lyric";
      case "topic": return "video.kind.topic";
      default: return "video.kind.video";
    }
  }
}

export const videoState = new VideoState();
