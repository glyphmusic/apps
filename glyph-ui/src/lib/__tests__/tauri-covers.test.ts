import { afterEach, describe, expect, it, vi } from "vitest";

const calls: string[] = [];
let answer: (cmd: string, track: { id: string }) => Promise<unknown>;
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args: { track: { id: string } }) => {
    calls.push(`${cmd}:${args.track.id}`);
    return answer(cmd, args.track);
  },
  convertFileSrc: (p: string) => `asset://${p}`,
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => {}) }));

import { TauriBackend, coverBusyMs, coverRetryDelay } from "../api/tauri";
import type { Track } from "../api/types";

const song = (id: string) => ({ id, source: "deezer", title: `Song ${id}`, artist: "Artist", album: null, cover_url: null }) as unknown as Track;

describe("tauri cover loading", () => {
  afterEach(() => {
    vi.useRealTimers();
    calls.length = 0;
  });

  it("reads the wait time the core asks for", () => {
    expect(coverBusyMs("cover_busy:4200")).toBe(4200);
    expect(coverBusyMs("something else")).toBeUndefined();
  });

  it("waits longer each time, at least as long as the server asked", () => {
    expect(coverRetryDelay(1, 0, 0)).toBe(2_000);
    expect(coverRetryDelay(2, 0, 0)).toBe(4_000);
    expect(coverRetryDelay(9, 0, 0)).toBe(30_000);
    expect(coverRetryDelay(1, 12_000, 0)).toBe(12_000);
  });

  it("shows a cover that is already on the device without asking the network", async () => {
    answer = async (cmd) => (cmd === "cover_local" ? "/covers/a.jpg" : null);
    const api = new TauriBackend();
    expect(await api.cover(song("a"))).toBe("asset:///covers/a.jpg");
    expect(calls).toEqual(["cover_local:a"]);
  });

  it("tries again quietly when the server is busy instead of answering no cover", async () => {
    vi.useFakeTimers();
    let busy = true;
    answer = async (cmd) => {
      if (cmd === "cover_local") return null;
      if (busy) throw "cover_busy:3000";
      return "/covers/b.jpg";
    };
    const api = new TauriBackend();
    let got: string | null | undefined;
    void api.cover(song("b")).then((u) => (got = u));
    await vi.advanceTimersByTimeAsync(10);
    expect(got).toBeUndefined();
    busy = false;
    await vi.advanceTimersByTimeAsync(5_000);
    expect(got).toBe("asset:///covers/b.jpg");
    expect(calls.filter((c) => c === "cover_path:b").length).toBe(2);
  });

  it("stops waiting when the row is scrolled away", async () => {
    vi.useFakeTimers();
    answer = async (cmd) => {
      if (cmd === "cover_local") return null;
      throw "cover_busy:60000";
    };
    const api = new TauriBackend();
    const ac = new AbortController();
    let got: string | null | undefined;
    void api.cover(song("c"), ac.signal).then((u) => (got = u));
    await vi.advanceTimersByTimeAsync(10);
    ac.abort();
    await vi.advanceTimersByTimeAsync(10);
    expect(got).toBeNull();
    await vi.advanceTimersByTimeAsync(70_000);
    expect(calls.filter((c) => c === "cover_path:c").length).toBe(1);
  });
});
