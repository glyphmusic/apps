// Crop dialog state (profile.rs CropDialog / profile_pick_image /
// profile_crop_apply / profile_remove_image). A picked picture waits in the
// dialog; "Apply" cuts the framed region out in a canvas (picture 512 x 512,
// banner 1500 x 500, WebP) and sends it with the command `profile_set_image`.
import { encodePicture, pictureCanvas } from "../../picture";
import { backend } from "../../api/backend";
import { app } from "../../state.svelte";
import { t, tf } from "../../i18n.svelte";

/** Longest zoom the slider offers. */
export const CROP_MAX_ZOOM = 4;
/** Crop window in the dialog (picture: square, banner: 3:1). */
export const CROP_AVATAR = 280;
export const CROP_BANNER: [number, number] = [432, 144];
/** Area around the crop window. */
export const CROP_STAGE: [number, number] = [480, 320];

export interface CropState {
  url: string;
  banner: boolean;
  w: number;
  h: number;
  zoom: number;
  cx: number;
  cy: number;
}

export const crop = $state<{ dlg: CropState | null; busy: boolean; notice: { text: string; ok: boolean } | null; version: number; host: symbol | null }>({
  dlg: null, busy: false, notice: null, version: 0, host: null,
});

export function cropWindow(banner: boolean): [number, number] {
  return banner ? CROP_BANNER : [CROP_AVATAR, CROP_AVATAR];
}

/** glyph_core::profile_images::crop_rect: region (x, y, w, h) in image pixels. */
export function cropRect(w: number, h: number, banner: boolean, c: { zoom: number; cx: number; cy: number }): [number, number, number, number] {
  const aspect = banner ? 3 : 1;
  const [bw, bh] = w >= h * aspect ? [h * aspect, h] : [w, w / aspect];
  const z = Math.max(1, c.zoom);
  const [cw, ch] = [Math.max(1, bw / z), Math.max(1, bh / z)];
  const x = Math.min(w - cw, Math.max(0, c.cx * w - cw / 2));
  const y = Math.min(h - ch, Math.max(0, c.cy * h - ch / 2));
  return [Math.floor(x), Math.floor(y), Math.min(w, Math.max(1, Math.floor(cw))), Math.min(h, Math.max(1, Math.floor(ch)))];
}

/** Screen pixels per image pixel. */
export function cropScale(d: CropState): number {
  const [, , cw] = cropRect(d.w, d.h, d.banner, d);
  return cropWindow(d.banner)[0] / Math.max(1, cw);
}

/** Keeps the crop region inside the image. */
export function clampCrop(d: CropState) {
  d.zoom = Math.min(CROP_MAX_ZOOM, Math.max(1, d.zoom));
  const [, , cw, ch] = cropRect(d.w, d.h, d.banner, d);
  const [hx, hy] = [cw / Math.max(1, d.w) / 2, ch / Math.max(1, d.h) / 2];
  d.cx = Math.min(1 - hx, Math.max(hx, d.cx));
  d.cy = Math.min(1 - hy, Math.max(hy, d.cy));
}

/** Error text of a failed profile call (profile.rs err_text). The backend
 *  rejects with a message that is one of: signed_out, not_found, too_big,
 *  rate_limited; anything else is generic. */
export function profileError(e: unknown): string {
  const m = String((e as Error)?.message ?? e ?? "");
  if (m.includes("signed_out")) return t("account.signed_out");
  if (m.includes("not_found") || m.includes("404")) return t("profile.not_found");
  if (m.includes("account_full")) return t("account.storage.full");
  if (m.includes("too_big") || m.includes("413")) return t("profile.image.too_big");
  if (m.includes("rate_limited")) return t("account.err.slow");
  return t("account.err.generic");
}

/** Opens the file picker; the picture goes into the crop dialog. */
export function pickProfileImage(banner: boolean) {
  if (crop.busy) return;
  const input = document.createElement("input");
  input.type = "file";
  input.accept = "image/jpeg,image/png,image/webp";
  input.onchange = () => {
    const file = input.files?.[0];
    if (!file) return;
    const url = URL.createObjectURL(file);
    const im = new Image();
    im.onload = () => {
      if (im.naturalWidth > 0 && im.naturalHeight > 0) {
        crop.dlg = { url, banner, w: im.naturalWidth, h: im.naturalHeight, zoom: 1, cx: 0.5, cy: 0.5 };
      } else {
        crop.notice = { text: tf("toast.cover_error", { e: "empty image" }), ok: false };
        URL.revokeObjectURL(url);
      }
    };
    im.onerror = () => {
      crop.notice = { text: tf("toast.cover_error", { e: "unreadable image" }), ok: false };
      URL.revokeObjectURL(url);
    };
    im.src = url;
  };
  input.click();
}

export function cancelCrop() {
  if (crop.dlg) URL.revokeObjectURL(crop.dlg.url);
  crop.dlg = null;
}

/** Cuts the framed region out and uploads it. */
export async function applyCrop() {
  const d = crop.dlg;
  if (!d) return;
  crop.dlg = null;
  crop.busy = true;
  try {
    const im = new Image();
    await new Promise<void>((ok, err) => { im.onload = () => ok(); im.onerror = () => err(new Error("image")); im.src = d.url; });
    const [x, y, w, h] = cropRect(d.w, d.h, d.banner, d);
    const [ow, oh] = d.banner ? [1500, 500] : [512, 512];
    const [cv, g] = pictureCanvas(ow, oh);
    g.drawImage(im, x, y, w, h, 0, 0, ow, oh);
    const data = encodePicture(cv, 0.86);
    await backend().call("profile_set_image", { banner: d.banner, image: data });
    crop.notice = null;
    crop.version++;
  } catch (e) {
    crop.notice = { text: e instanceof Error && e.message === "image" ? tf("toast.cover_error", { e: "unreadable image" }) : profileError(e), ok: false };
    app.toastInfo(crop.notice.text);
  } finally {
    URL.revokeObjectURL(d.url);
    crop.busy = false;
  }
}

/** Removes the picture or the banner. */
export async function removeProfileImage(banner: boolean) {
  try {
    await backend().call("profile_set_image", { banner, image: null });
    crop.version++;
  } catch (e) {
    app.toastInfo(profileError(e));
  }
}
