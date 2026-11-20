// Background pictures (Settings › Appearance › Background): the user's own
// and those plugins' looks bring (glyph_app::backgrounds). The one picked is
// `config.visuals.bg_image`; Background.svelte draws it instead of the glow.
import { backend } from "./api/backend";
import { app } from "./state.svelte";
import { t } from "./i18n.svelte";
import { errText } from "./settings/util";

export interface BgItem { id: string; url: string; plugin: string | null }

/** Longest side a picture is stored with (sharp on 4K screens). */
const MAX_SIDE = 3840;
/** Pictures smaller than this (and not wider than MAX_SIDE) are stored as they are. */
const KEEP_BYTES = 3 * 1024 * 1024;

export const bgs = $state({ items: [] as BgItem[], loaded: false, adding: false });

let loading: Promise<void> | null = null;

/** Reads the list (again after plugins changed). */
export function loadBackgrounds(): Promise<void> {
  loading ??= backend()
    .call<{ items: BgItem[] } | undefined>("backgrounds_list")
    .then((r) => { bgs.items = r?.items ?? []; bgs.loaded = true; })
    .catch(() => { bgs.loaded = true; })
    .finally(() => { loading = null; });
  return loading;
}

/** Address of picture `id`, if it is (still) there. */
export function bgUrl(id: string | undefined | null): string | null {
  if (!id) return null;
  return bgs.items.find((b) => b.id === id)?.url ?? null;
}

export function setBackground(id: string) {
  return app.setConfig((c) => { c.visuals.bg_image = id; });
}

/** Lets the user pick a picture file, stores it and shows it at once. */
export function pickBackground() {
  const input = document.createElement("input");
  input.type = "file";
  input.accept = "image/png,image/jpeg,image/webp";
  input.onchange = () => {
    const f = input.files?.[0];
    if (f) void addBackground(f);
  };
  input.click();
}

export async function addBackground(file: File) {
  if (bgs.adding) return;
  bgs.adding = true;
  try {
    const data = await toBase64(await shrink(file));
    const id = await backend().call<string>("background_add", { data });
    await loadBackgrounds();
    if (id) await setBackground(id);
  } catch (e) {
    app.toastInfo(errText(e) || t("bg.add_failed"));
  }
  bgs.adding = false;
}

/** Deletes one of the user's own pictures; the glow comes back if it was shown. */
export async function removeBackground(id: string) {
  if (app.config.visuals.bg_image === id) await setBackground("");
  try {
    await backend().call("background_remove", { id });
  } catch (e) {
    app.toastInfo(errText(e));
  }
  await loadBackgrounds();
}

/** The picture scaled down to MAX_SIDE (WebP, else JPEG), or as it is when already small. */
async function shrink(file: File): Promise<Blob> {
  let bmp: ImageBitmap;
  try {
    bmp = await createImageBitmap(file);
  } catch {
    throw new Error(t("bg.not_picture"));
  }
  const scale = Math.min(1, MAX_SIDE / Math.max(bmp.width, bmp.height));
  const ok = ["image/png", "image/jpeg", "image/webp"].includes(file.type);
  if (scale === 1 && ok && file.size <= KEEP_BYTES) { bmp.close(); return file; }
  const w = Math.max(1, Math.round(bmp.width * scale)), h = Math.max(1, Math.round(bmp.height * scale));
  const c = document.createElement("canvas");
  c.width = w;
  c.height = h;
  const x = c.getContext("2d")!;
  x.imageSmoothingQuality = "high";
  x.drawImage(bmp, 0, 0, w, h);
  bmp.close();
  const enc = (type: string, q: number) => new Promise<Blob | null>((r) => c.toBlob(r, type, q));
  const webp = await enc("image/webp", 0.9);
  // Some engines write PNG when they can't write WebP: JPEG is smaller then.
  if (webp?.type === "image/webp") return webp;
  const jpg = await enc("image/jpeg", 0.9);
  if (!jpg) throw new Error(t("bg.add_failed"));
  return jpg;
}

async function toBase64(b: Blob): Promise<string> {
  const bytes = new Uint8Array(await b.arrayBuffer());
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

/** Average brightness (0..1) of a picture, null when it can't be read. */
export async function pictureLuma(url: string): Promise<number | null> {
  try {
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.src = url;
    await img.decode();
    const c = document.createElement("canvas");
    c.width = c.height = 24;
    const x = c.getContext("2d", { willReadFrequently: true })!;
    x.drawImage(img, 0, 0, 24, 24);
    const d = x.getImageData(0, 0, 24, 24).data;
    let sum = 0;
    for (let i = 0; i < d.length; i += 4) sum += (0.2126 * d[i] + 0.7152 * d[i + 1] + 0.0722 * d[i + 2]) / 255;
    return sum / (d.length / 4);
  } catch {
    return null;
  }
}
