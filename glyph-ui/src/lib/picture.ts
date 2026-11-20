// Pictures the user picks (playlist covers, profile picture and banner).
// They are written as small lossy WebP: that is what the server keeps, and
// an opaque canvas makes the plain WebP format (no metadata, no alpha) that
// the server stores as sent. Engines that cannot write WebP get JPEG.

/** An opaque canvas of `w` x `h` and its context, smoothing on high. */
export function pictureCanvas(w: number, h: number): [HTMLCanvasElement, CanvasRenderingContext2D] {
  const c = document.createElement("canvas");
  c.width = w;
  c.height = h;
  const g = c.getContext("2d", { alpha: false })!;
  g.imageSmoothingQuality = "high";
  return [c, g];
}

/** The canvas as a data URL: WebP, or JPEG where WebP cannot be written. */
export function encodePicture(c: HTMLCanvasElement, quality = 0.82): string {
  const webp = c.toDataURL("image/webp", quality);
  return webp.startsWith("data:image/webp") ? webp : c.toDataURL("image/jpeg", quality);
}

/** Opens the file picker; resolves with a centre-cropped square picture
 *  (`size` px, data URL), or null when the user picks nothing. */
export function pickSquarePicture(size = 512): Promise<string | null> {
  return new Promise((done, fail) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "image/jpeg,image/png,image/webp,image/gif";
    input.addEventListener("cancel", () => done(null));
    input.onchange = () => {
      const file = input.files?.[0];
      if (!file) return done(null);
      const url = URL.createObjectURL(file);
      const im = new Image();
      im.onload = () => {
        URL.revokeObjectURL(url);
        const s = Math.min(im.naturalWidth, im.naturalHeight);
        if (!s) return fail(new Error("empty image"));
        const [c, g] = pictureCanvas(size, size);
        g.drawImage(im, (im.naturalWidth - s) / 2, (im.naturalHeight - s) / 2, s, s, 0, 0, size, size);
        done(encodePicture(c));
      };
      im.onerror = () => { URL.revokeObjectURL(url); fail(new Error("unreadable image")); };
      im.src = url;
    };
    input.click();
  });
}
