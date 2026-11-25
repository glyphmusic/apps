import { describe, expect, it } from "vitest";
import type { Library } from "../api/types";
import { cleanFolderName, createFolder, deleteFolder, folderOf, folderPlaylists, renameFolder, setPlaylistFolder } from "../folders";

const lib = (): Library => ({
  liked: [], recent: [], saved_albums: [], pinned: [], recent_lists: [], followed: [], downloads: [], search_history: [], hidden_sidebar: [],
  playlists: [
    { id: "a", name: "A", tracks: [] },
    { id: "b", name: "B", tracks: [] },
  ] as unknown as Library["playlists"],
});

describe("library folders", () => {
  it("moves playlists between folders like the core", () => {
    const l = lib();
    createFolder(l, "f1", "  Rock  ", ["a"]);
    expect(l.folders![0].name).toBe("Rock");
    expect(folderOf(l, "a")?.id).toBe("f1");
    createFolder(l, "f2", "Chill");
    setPlaylistFolder(l, "a", "f2");
    setPlaylistFolder(l, "b", "f2");
    expect(l.folders![0].playlists).toEqual([]);
    expect(folderPlaylists(l, "f2").map((p) => p.id)).toEqual(["a", "b"]);
    setPlaylistFolder(l, "a", "nope");
    expect(folderOf(l, "a")?.id).toBe("f2");
    setPlaylistFolder(l, "a", null);
    expect(folderOf(l, "a")).toBeUndefined();
    renameFolder(l, "f2", "Calm");
    expect(l.folders![1].name).toBe("Calm");
    deleteFolder(l, "f2");
    expect(folderOf(l, "b")).toBeUndefined();
    expect(l.playlists).toHaveLength(2);
  });

  it("skips playlists that no longer exist", () => {
    const l = lib();
    createFolder(l, "f", "X", ["a", "gone"]);
    expect(folderPlaylists(l, "f").map((p) => p.id)).toEqual(["a"]);
    expect(cleanFolderName("x".repeat(150))).toHaveLength(100);
  });
});
