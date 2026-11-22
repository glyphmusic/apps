// Whether the signed-in account may open the server dashboard. Checked via
// `GET /admin/v1/me`; transient failures (e.g. backend still connecting on
// startup or temporary offline) do not permanently suppress the entry and are
// retried automatically.
import { backend } from "../api/backend";
import type { AdminMe } from "./api";

/** The dashboard's sub pages ("" = overview): /server/people and so on. */
export type AdminSub = "" | "people" | "settings" | "versions";
export const ADMIN_SUBS: AdminSub[] = ["people", "settings", "versions"];

/** Delays (ms) for automatic retries after a transient failure. */
export const RETRY_DELAYS_MS = [1500, 4000, 10_000];
/** Minimum gap (ms) before re-checking a previously settled answer on resume/focus. */
export const RESUME_RECHECK_MS = 15_000;

function errorCode(e: unknown): string {
  return String((e as { message?: unknown })?.message ?? e);
}

function isDefinitiveDeny(code: string): boolean {
  return code === "not_admin" || code === "signed_out";
}

export class AdminState {
  me = $state<AdminMe | null>(null);
  /** Still asking the server (only true when we have no admin session yet). */
  pending = $state(false);
  /** Open sub page; back returns to the overview first. */
  sub = $state<AdminSub>("");
  /** Bumped by the refresh buttons: the page loads again. */
  reload = $state(0);

  /** Signed-in handle this state tracks ("" = signed out). */
  private handle = "";
  /** Handle that received a definitive server answer (admin or not_admin/signed_out). */
  private settled = "";
  /** Monotonic request ID so stale/superseded responses are ignored. */
  private seq = 0;
  /** Handle currently being fetched ("" = idle). */
  private inFlight = "";
  /** Timestamp (ms) of the last completed definitive check. */
  private checkedAt = 0;
  /** Number of consecutive transient retries scheduled for the current handle. */
  private retries = 0;
  private retryTimer: ReturnType<typeof setTimeout> | null = null;
  private readonly fetchMe: () => Promise<AdminMe | null | undefined>;

  constructor(fetchMe?: () => Promise<AdminMe | null | undefined>) {
    this.fetchMe = fetchMe ?? (() => backend().call<AdminMe>("admin_call", { method: "GET", path: "me" }));
  }

  get on(): boolean { return !!this.me; }

  private clearRetry() {
    if (this.retryTimer !== null) {
      clearTimeout(this.retryTimer);
      this.retryTimer = null;
    }
  }

  private resetFor(h: string) {
    this.seq++;
    this.handle = h;
    this.settled = "";
    this.inFlight = "";
    this.checkedAt = 0;
    this.retries = 0;
    this.clearRetry();
    this.me = null;
    this.pending = false;
  }

  /**
   * Call when the signed-in account or backend readiness may have changed.
   * When `ready` is false (backend still connecting/offline), waits instead of
   * firing a doomed request while still clearing state on sign-out.
   */
  check(handle: string | null | undefined, ready = true) {
    const h = handle ?? "";
    if (h !== this.handle) {
      this.resetFor(h);
    }
    if (!h || !ready) return;
    if (this.settled === h || this.inFlight === h) return;
    this.run(h);
  }

  /**
   * Re-check admin status for `handle` even if we already got an answer before
   * (e.g. backend came up / reconnected, or role may have been granted while
   * the app was open). When `minAgeMs > 0` and we already have a definitive
   * answer for `h`, skips if checked more recently than `minAgeMs`.
   */
  refresh(handle: string | null | undefined, minAgeMs = 0) {
    const h = handle ?? "";
    if (h !== this.handle) {
      this.resetFor(h);
    }
    if (!h) return;
    if (minAgeMs > 0 && this.settled === h && Date.now() - this.checkedAt < minAgeMs) {
      return;
    }
    if (minAgeMs > 0 && this.inFlight === h) return;
    this.retries = 0;
    this.clearRetry();
    this.run(h);
  }

  /** Re-check on app foreground/focus/online if unsettled or stale. */
  onResume(handle: string | null | undefined, ready = true) {
    const h = handle ?? "";
    if (!h) {
      if (this.handle) this.resetFor("");
      return;
    }
    if (!ready) return;
    this.refresh(h, this.settled === h ? RESUME_RECHECK_MS : 0);
  }

  private run(h: string) {
    this.clearRetry();
    const id = ++this.seq;
    this.inFlight = h;
    if (!this.me) this.pending = true;
    this.fetchMe()
      .then((m) => {
        if (this.seq !== id || this.handle !== h) return;
        this.me = m ?? null;
        this.settled = h;
        this.checkedAt = Date.now();
        this.retries = 0;
      })
      .catch((e) => {
        if (this.seq !== id || this.handle !== h) return;
        const code = errorCode(e);
        if (isDefinitiveDeny(code)) {
          this.me = null;
          this.settled = h;
          this.checkedAt = Date.now();
          this.retries = 0;
        } else {
          // Transient error (backend_offline, offline, network hiccup, 5xx):
          // keep any existing `me`, do not lock out retries, and try again soon.
          this.scheduleRetry(h, id);
        }
      })
      .finally(() => {
        if (this.seq !== id) return;
        this.inFlight = "";
        this.pending = false;
      });
  }

  private scheduleRetry(h: string, id: number) {
    const delay = RETRY_DELAYS_MS[this.retries];
    if (delay === undefined) return;
    this.retries++;
    this.clearRetry();
    this.retryTimer = setTimeout(() => {
      this.retryTimer = null;
      if (this.seq !== id || this.handle !== h || this.settled === h) return;
      this.run(h);
    }, delay);
  }

  /** The role was taken away while the dashboard was open. */
  lost() {
    this.me = null;
    this.clearRetry();
    if (this.handle) {
      this.settled = this.handle;
      this.checkedAt = Date.now();
    }
  }
}

export const admin = new AdminState();

/** One admin API call; errors are short codes (`not_admin`, `owner_only`, `bad_request: …`). */
export async function adminCall<T>(method: "GET" | "POST" | "PUT", path: string, body?: unknown): Promise<T> {
  try {
    return await backend().call<T>("admin_call", { method, path, body: body ?? null });
  } catch (e) {
    const code = errorCode(e);
    if (code === "not_admin" || code === "signed_out") admin.lost();
    throw new Error(code);
  }
}
