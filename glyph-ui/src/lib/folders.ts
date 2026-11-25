// Library folders (glyph_core::library LibraryFolder): named groups of own
// playlists. No nesting; a playlist sits in at most one folder. The same
// rules as the Rust core, for the web and demo backends and the UI.
import type { Library, LibraryFolder } from "./api/types";

const NAME_MAX = 100;

export function cleanFolderName(name: string): string {
  return [...name.trim()].slice(0, NAME_MAX).join("").trimEnd();
}

export function folderOf(lib: Pick<Library, "folders">, playlist: string): LibraryFolder | undefined {
  return (lib.folders ?? []).find((f) => f.playlists.includes(playlist));
}

/** Moves a playlist into `folder` (at its end) or out of every folder with null. */
export function setPlaylistFolder(lib: Library, playlist: string, folder: string | null): void {
  const folders = (lib.folders ??= []);
  if (folder && !folders.some((f) => f.id === folder)) return;
  if (folder && folderOf(lib, playlist)?.id === folder) return;
  for (const f of folders) f.playlists = f.playlists.filter((p) => p !== playlist);
  if (folder) folders.find((f) => f.id === folder)!.playlists.push(playlist);
}

export function createFolder(lib: Library, id: string, name: string, playlists: string[] = []): string {
  (lib.folders ??= []).push({ id, name: cleanFolderName(name), playlists: [] });
  for (const p of playlists) setPlaylistFolder(lib, p, id);
  return id;
}

export function renameFolder(lib: Library, id: string, name: string): void {
  const f = lib.folders?.find((x) => x.id === id);
  if (f) f.name = cleanFolderName(name);
}

/** Removes the folder; its playlists go back to the top of the library. */
export function deleteFolder(lib: Library, id: string): void {
  lib.folders = (lib.folders ?? []).filter((f) => f.id !== id);
}

/** Playlists of a folder that still exist, in folder order. */
export function folderPlaylists(lib: Pick<Library, "folders" | "playlists">, id: string) {
  const f = lib.folders?.find((x) => x.id === id);
  if (!f) return [];
  return f.playlists.map((pid) => lib.playlists.find((p) => p.id === pid)).filter((p) => !!p);
}

/** Backend handlers shared by the web and demo backends. */
export function folderHandlers(lib: () => Library, save: () => void) {
  return {
    folder_create: (a: Record<string, any>) => { const id = createFolder(lib(), `f-${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`, a.name, a.playlists ?? []); save(); return id; },
    folder_rename: (a: Record<string, any>) => { renameFolder(lib(), a.id, a.name); save(); },
    folder_delete: (a: Record<string, any>) => { deleteFolder(lib(), a.id); save(); },
    folder_set: (a: Record<string, any>) => { setPlaylistFolder(lib(), a.playlist, a.folder ?? null); save(); },
  };
}
