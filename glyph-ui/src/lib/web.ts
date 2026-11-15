// The UI as the web player (glyph.my/web): what only the app can do is
// hidden, with a short "Available in the app" hint and a download link.
import { backend } from "./api/backend";

/** Where people get the app. */
export const GET_APP_URL = "https://glyph.my/download";

/** The UI runs in a plain browser as the web player. */
export function onWeb(): boolean {
  try { return backend().kind === "web"; } catch { return false; }
}
