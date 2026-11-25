// Demo handlers for library folders (glyph_core::library LibraryFolder):
//
//   folder_create { name, playlists?: string[] }  -> string (new folder id; the playlists move into it)
//   folder_rename { id, name }                    -> void
//   folder_delete { id }                          -> void   (its playlists go back to the library)
//   folder_set    { playlist, folder: string | null } -> void (move into a folder, or out with null)
import type { DemoHandler } from "../demo";
import { folderHandlers } from "../../folders";

let ctxRef: Parameters<DemoHandler>[0] | null = null;
const shared = folderHandlers(() => ctxRef!.library, () => ctxRef!.saveLibrary());
const wrap = (f: (a: Record<string, any>) => unknown): DemoHandler => (ctx, a) => { ctxRef = ctx; return f(a); };

export const handlers: Record<string, DemoHandler> = Object.fromEntries(
  Object.entries(shared).map(([k, f]) => [k, wrap(f)]),
);
