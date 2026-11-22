import { afterEach, describe, expect, it, vi } from "vitest";
import { AdminState, RESUME_RECHECK_MS, RETRY_DELAYS_MS } from "../admin/admin.svelte";
import type { AdminMe } from "../admin/api";

const ADMIN_ME: AdminMe = { owner: false, handle: "alice", keys: false, updates: false, roles: false };

async function flushMicrotasks() {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

describe("AdminState", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("waits until backend is ready instead of locking out on startup", async () => {
    const fetchMe = vi.fn<() => Promise<AdminMe>>().mockResolvedValue(ADMIN_ME);
    const state = new AdminState(fetchMe);

    // Account loaded from disk while backend is still connecting.
    state.check("alice", false);
    expect(fetchMe).not.toHaveBeenCalled();
    expect(state.on).toBe(false);

    // Backend becomes ready.
    state.check("alice", true);
    expect(fetchMe).toHaveBeenCalledTimes(1);
    await flushMicrotasks();
    expect(state.on).toBe(true);
    expect(state.me).toEqual(ADMIN_ME);
  });

  it("retries automatically after transient startup/offline errors", async () => {
    vi.useFakeTimers();
    const fetchMe = vi
      .fn<() => Promise<AdminMe>>()
      .mockRejectedValueOnce("backend_offline")
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValueOnce(ADMIN_ME);
    const state = new AdminState(fetchMe);

    state.check("alice", true);
    await flushMicrotasks();
    expect(fetchMe).toHaveBeenCalledTimes(1);
    expect(state.on).toBe(false);

    // First scheduled retry.
    await vi.advanceTimersByTimeAsync(RETRY_DELAYS_MS[0]);
    await flushMicrotasks();
    expect(fetchMe).toHaveBeenCalledTimes(2);
    expect(state.on).toBe(false);

    // Second scheduled retry succeeds.
    await vi.advanceTimersByTimeAsync(RETRY_DELAYS_MS[1]);
    await flushMicrotasks();
    expect(fetchMe).toHaveBeenCalledTimes(3);
    expect(state.on).toBe(true);
  });

  it("stops retrying on definitive not_admin, but picks up newly granted role on resume/refresh", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-11-22T20:00:00Z"));
    const fetchMe = vi
      .fn<() => Promise<AdminMe>>()
      .mockRejectedValueOnce("not_admin")
      .mockResolvedValueOnce(ADMIN_ME);
    const state = new AdminState(fetchMe);

    state.check("alice", true);
    await flushMicrotasks();
    expect(fetchMe).toHaveBeenCalledTimes(1);
    expect(state.on).toBe(false);

    // No automatic retry should fire after a definitive denial.
    await vi.advanceTimersByTimeAsync(60_000);
    expect(fetchMe).toHaveBeenCalledTimes(1);

    // Resuming within cooldown after another check stays throttled, but after RESUME_RECHECK_MS it re-checks.
    state.onResume("alice", true);
    await flushMicrotasks();
    expect(fetchMe).toHaveBeenCalledTimes(2);
    expect(state.on).toBe(true);

    // Immediately resuming again is throttled.
    state.onResume("alice", true);
    await flushMicrotasks();
    expect(fetchMe).toHaveBeenCalledTimes(2);

    // Advancing less than RESUME_RECHECK_MS stays throttled.
    await vi.advanceTimersByTimeAsync(RESUME_RECHECK_MS - 1000);
    state.onResume("alice", true);
    await flushMicrotasks();
    expect(fetchMe).toHaveBeenCalledTimes(2);
  });

  it("ignores stale in-flight responses when signing out or switching accounts", async () => {
    let resolveFirst!: (v: AdminMe) => void;
    const first = new Promise<AdminMe>((r) => { resolveFirst = r; });
    const fetchMe = vi
      .fn<() => Promise<AdminMe>>()
      .mockReturnValueOnce(first)
      .mockRejectedValueOnce("not_admin");
    const state = new AdminState(fetchMe);

    state.check("alice", true);
    expect(state.pending).toBe(true);

    // Sign out while alice's request is still in flight.
    state.check(null, true);
    expect(state.on).toBe(false);
    expect(state.pending).toBe(false);

    // Stale response from alice arrives late and must be ignored.
    resolveFirst(ADMIN_ME);
    await flushMicrotasks();
    expect(state.on).toBe(false);
    expect(state.me).toBeNull();

    // Switching to non-admin bob checks bob cleanly.
    state.check("bob", true);
    await flushMicrotasks();
    expect(fetchMe).toHaveBeenCalledTimes(2);
    expect(state.on).toBe(false);
  });

  it("keeps existing admin state on transient refresh error and clears it on revocation", async () => {
    const fetchMe = vi
      .fn<() => Promise<AdminMe>>()
      .mockResolvedValueOnce(ADMIN_ME)
      .mockRejectedValueOnce("offline")
      .mockRejectedValueOnce("not_admin");
    const state = new AdminState(fetchMe);

    state.check("alice", true);
    await flushMicrotasks();
    expect(state.on).toBe(true);

    // Transient offline error on refresh does not hide the button.
    state.refresh("alice");
    await flushMicrotasks();
    expect(state.on).toBe(true);

    // Definitive revocation hides the button.
    state.refresh("alice");
    await flushMicrotasks();
    expect(state.on).toBe(false);
  });
});
