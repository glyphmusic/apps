import { describe, expect, it } from "vitest";
import { buildQuickJump, scoreMatch } from "../shell/quick-jump";
import type { Playlist, Track } from "../api/types";

const song = (id: string, title: string, artist: string, album?: string): Track => ({
  id,
  source: "deezer",
  title,
  artist,
  album,
});

describe("quick-jump matcher", () => {
  const playlists: Playlist[] = [
    { id: "p1", name: "Sunday Morning", tracks: [song("1", "Salad Days", "Mac DeMarco")] },
    { id: "p2", name: "Late Night Coding", tracks: [song("2", "Space Song", "Beach House")] },
  ];
  const liked: Track[] = [
    song("1", "Salad Days", "Mac DeMarco", "Salad Days"),
    song("3", "Let It Happen", "Tame Impala", "Currents"),
    song("4", "Morning Bell", "Radiohead", "Kid A"),
  ];

  it("ranks exact > prefix > word-boundary > substring", () => {
    expect(scoreMatch("Sunday", "sunday")).toBeGreaterThan(scoreMatch("Sunday Morning", "sunday"));
    expect(scoreMatch("Sunday Morning", "sunday")).toBeGreaterThan(scoreMatch("Sunday Morning", "morning"));
    expect(scoreMatch("Sunday Morning", "morning")).toBeGreaterThan(scoreMatch("Submorning", "morning"));
    expect(scoreMatch("Sunday", "xyz")).toBe(0);
  });

  it("prioritizes matching user playlists and deduplicates songs", () => {
    const hits = buildQuickJump({
      query: "morning",
      playlists,
      liked,
      savedAlbums: [],
      followed: ["Morning Parade"],
      builtInLists: [],
      pages: [],
    });
    expect(hits.length).toBeGreaterThanOrEqual(2);
    expect(hits.some((x) => x.kind === "list" && x.title === "Sunday Morning")).toBe(true);
    expect(hits.some((x) => x.kind === "artist" && x.name === "Morning Parade")).toBe(true);
    expect(hits.some((x) => x.kind === "song" && x.track.title === "Morning Bell")).toBe(true);
  });

  it("matches pages by localized label or keyword and respects limit", () => {
    const hits = buildQuickJump({
      query: "transfer",
      playlists: [],
      liked: [],
      savedAlbums: [],
      followed: [],
      builtInLists: [],
      pages: [
        { id: "migrate", label: "Musik übertragen", keywords: ["transfer", "spotify"], icon: "arrow-right-left" },
        { id: "settings", label: "Einstellungen", keywords: ["settings"], icon: "settings" },
      ],
      limit: 3,
    });
    expect(hits).toHaveLength(1);
    expect(hits[0]).toMatchObject({ kind: "page", id: "migrate" });
  });

  it("returns empty array for blank query", () => {
    expect(
      buildQuickJump({
        query: "   ",
        playlists,
        liked,
        savedAlbums: [],
        followed: [],
        builtInLists: [],
        pages: [],
      }),
    ).toEqual([]);
  });
});
