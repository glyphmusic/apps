// Temporary ("draft") playlists created from "+ New playlist" inside
// destination pickers (Transfer music, By link, Add shared). They behave like
// selectable playlists in the picker, open the standard NameDialog modal for
// naming, and disappear automatically as soon as nothing references them —
// so unused empty playlists are never left behind in the library.
import { app } from "./state.svelte";
import { t, tf } from "./i18n.svelte";

export interface DraftPlaylist {
  id: string;
  name: string;
}

export const DRAFT_PREFIX = "draft:";

export function isDraftValue(value: string): boolean {
  return value.startsWith(DRAFT_PREFIX);
}

export function draftIdOf(value: string): string | null {
  return value.startsWith(DRAFT_PREFIX) ? value.slice(DRAFT_PREFIX.length) : null;
}

export function draftValue(id: string): string {
  return `${DRAFT_PREFIX}${id}`;
}

let nextSeq = 0;

export function createDraftPlaylists() {
  let items = $state<DraftPlaylist[]>([]);

  function get(id: string): DraftPlaylist | undefined {
    return items.find((d) => d.id === id);
  }

  function create(name: string): DraftPlaylist {
    const clean = name.trim() || tf("library.new_playlist_name", { n: app.library.playlists.length + items.length + 1 });
    const draft: DraftPlaylist = { id: `draft-${++nextSeq}`, name: clean };
    items = [draft, ...items];
    return draft;
  }

  function rename(id: string, name: string): void {
    const clean = name.trim();
    if (!clean) return;
    items = items.map((d) => (d.id === id ? { ...d, name: clean } : d));
  }

  function remove(id: string): void {
    items = items.filter((d) => d.id !== id);
  }

  /** Keeps only draft playlists whose ids are still referenced; drops the rest. */
  function prune(usedIds: Iterable<string>): void {
    const keep = usedIds instanceof Set ? usedIds : new Set(usedIds);
    if (items.every((d) => keep.has(d.id))) return;
    items = items.filter((d) => keep.has(d.id));
  }

  function clear(): void {
    if (items.length) items = [];
  }

  /** Opens the NameDialog modal to create a new draft playlist. */
  function promptCreate(opts: { initial?: string; onCreated: (draft: DraftPlaylist) => void }): void {
    app.ctx = null;
    app.controlMenu = null;
    app.nameDialog = {
      tracks: [],
      rename: null,
      initial: opts.initial ?? "",
      fallback: tf("library.new_playlist_name", { n: app.library.playlists.length + items.length + 1 }),
      onSubmit: (nm) => {
        const draft = create(nm);
        opts.onCreated(draft);
      },
    };
  }

  /** Opens the NameDialog modal to rename an existing draft playlist. */
  function promptRename(id: string): void {
    const cur = get(id);
    if (!cur) return;
    app.ctx = null;
    app.controlMenu = null;
    app.nameDialog = {
      tracks: [],
      rename: null,
      initial: cur.name,
      title: t("migrate.rename"),
      okLabel: t("dialog.save"),
      nonEmpty: true,
      onSubmit: (nm) => rename(id, nm),
    };
  }

  return {
    get items() {
      return items;
    },
    get,
    create,
    rename,
    remove,
    prune,
    clear,
    promptCreate,
    promptRename,
  };
}
