// app/lists.rs album_label: "2019 · EP" for an album, looked up once per
// album (backend `album_meta`), guessed from the song count until (or
// unless) the catalog knows it.
import { backend } from "../../api/backend";
import type { ReleaseKind } from "../../api/types";
import { t } from "../../i18n.svelte";
import { kindFromCount } from "./actions";

type Meta = { kind: ReleaseKind; year?: number | null } | null;
const metas = $state<Record<string, Meta>>({});
const asked = new Set<string>();

export function cachedAlbumKind(title: string, artist: string): ReleaseKind | undefined {
  return metas[`${title.toLowerCase()}|${artist.toLowerCase()}`]?.kind;
}

export function albumLabel(title: string, artist: string, n: number): string {
  const key = `${title.toLowerCase()}|${artist.toLowerCase()}`;
  if (!asked.has(key)) {
    asked.add(key);
    if (asked.size > 512) asked.clear();
    backend().call<Meta>("album_meta", { title, artist }).then((m) => { if (m) metas[key] = m; });
  }
  const m = metas[key];
  const kind = m ? t(`album.kind.${m.kind}`) : kindFromCount(n);
  return m?.year ? `${m.year} · ${kind}` : kind;
}
