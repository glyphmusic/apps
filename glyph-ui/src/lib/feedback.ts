// Quick feedback helpers: GitHub issue URL builder and phone shake detector.
// On Android, MainActivity's native accelerometer calls `window.__glyphShake`;
// on mobile web / PWA, `devicemotion` feeds the same multi-peak detector.

export const FEEDBACK_ISSUES_URL = "https://github.com/glyphmusic/apps/issues/new";

const GRAVITY = 9.80665;

export interface FeedbackMeta {
  version?: string;
  platform?: string;
}

/** Builds a GitHub "new issue" URL pre-filled with the user's feedback text. */
export function buildFeedbackIssueUrl(message: string, meta?: FeedbackMeta): string {
  const text = message.trim();
  if (!text) return FEEDBACK_ISSUES_URL;

  const firstLine = text
    .split(/\r?\n/)
    .map((l) => l.trim())
    .find((l) => l.length > 0) ?? "";
  const compactTitle = firstLine.replace(/\s+/g, " ");
  const title = compactTitle.length > 72 ? `${compactTitle.slice(0, 71).trimEnd()}…` : compactTitle;

  const envParts: string[] = [];
  if (meta?.version && meta.version !== "0.0.0") envParts.push(`Glyph v${meta.version}`);
  if (meta?.platform) envParts.push(meta.platform);
  const body = envParts.length ? `${text}\n\n---\n${envParts.join(" · ")}` : text;

  const params = new URLSearchParams();
  if (title) params.set("title", title);
  params.set("body", body);
  return `${FEEDBACK_ISSUES_URL}?${params.toString()}`;
}

export interface ShakeOptions {
  /** Linear acceleration delta (in g) required to count a shake peak. */
  thresholdG?: number;
  /** Number of peaks needed within `windowMs` to confirm an intentional shake. */
  peaks?: number;
  /** Sliding window (ms) in which `peaks` must occur. */
  windowMs?: number;
  /** Minimum spacing (ms) between consecutive counted peaks. */
  minPeakGapMs?: number;
  /** Cooldown (ms) after triggering before another shake can fire. */
  cooldownMs?: number;
}

export interface ShakeDetector {
  sample(ax: number, ay: number, az: number, now?: number, includesGravity?: boolean): boolean;
  reset(): void;
}

/**
 * Multi-peak shake detector: ignores resting gravity and single bumps/taps,
 * firing only when several sharp acceleration peaks occur within a short burst.
 */
export function createShakeDetector(onShake: () => void, opts: ShakeOptions = {}): ShakeDetector {
  const thresholdG = opts.thresholdG ?? 1.35;
  const requiredPeaks = opts.peaks ?? 3;
  const windowMs = opts.windowMs ?? 900;
  const minPeakGapMs = opts.minPeakGapMs ?? 110;
  const cooldownMs = opts.cooldownMs ?? 2500;

  let peakTimes: number[] = [];
  let lastPeakAt = -Infinity;
  let lastTriggerAt = -Infinity;

  return {
    sample(ax: number, ay: number, az: number, now = Date.now(), includesGravity = true): boolean {
      if (!Number.isFinite(ax) || !Number.isFinite(ay) || !Number.isFinite(az)) return false;
      if (now - lastTriggerAt < cooldownMs) return false;
      const magG = Math.hypot(ax, ay, az) / GRAVITY;
      const linearG = includesGravity ? Math.abs(magG - 1) : magG;
      if (linearG < thresholdG) return false;
      if (now - lastPeakAt < minPeakGapMs) return false;

      lastPeakAt = now;
      peakTimes = peakTimes.filter((t) => now - t <= windowMs);
      peakTimes.push(now);

      if (peakTimes.length >= requiredPeaks) {
        peakTimes = [];
        lastTriggerAt = now;
        onShake();
        return true;
      }
      return false;
    },
    reset() {
      peakTimes = [];
      lastPeakAt = -Infinity;
    },
  };
}

/**
 * Listens for phone shakes via Android's native bridge (`window.__glyphShake`)
 * and the web `devicemotion` API (mobile browsers / PWAs).
 */
export function installShakeFeedback(onShake: () => void): () => void {
  if (typeof window === "undefined") return () => {};
  const win = window as unknown as {
    __glyphShake?: () => void;
    __GLYPH_ANDROID__?: unknown;
  };
  let lastNativeAt = -Infinity;
  const trigger = () => {
    const now = Date.now();
    if (now - lastNativeAt < 2000) return;
    lastNativeAt = now;
    onShake();
  };
  win.__glyphShake = trigger;

  // Native Android drives shake detection through SensorManager so WebView
  // throttling never drops shakes; skip redundant devicemotion listener there.
  if (win.__GLYPH_ANDROID__) {
    return () => {
      if (win.__glyphShake === trigger) delete win.__glyphShake;
    };
  }

  const detector = createShakeDetector(trigger);
  const onMotion = (e: DeviceMotionEvent) => {
    const lin = e.acceleration;
    if (lin && lin.x != null && lin.y != null && lin.z != null) {
      detector.sample(lin.x, lin.y, lin.z, Date.now(), false);
      return;
    }
    const grav = e.accelerationIncludingGravity;
    if (grav && grav.x != null && grav.y != null && grav.z != null) {
      detector.sample(grav.x, grav.y, grav.z, Date.now(), true);
    }
  };
  window.addEventListener("devicemotion", onMotion);
  return () => {
    window.removeEventListener("devicemotion", onMotion);
    if (win.__glyphShake === trigger) delete win.__glyphShake;
  };
}
