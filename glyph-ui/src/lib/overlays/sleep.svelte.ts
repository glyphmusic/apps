// app/sleep.rs for the UI: the sleep timer pauses playback after a chosen
// time or at the end of the current song. Not saved: a restart forgets it.
// `app.sleep` is what the player bar / Now Playing show. The Rust side pauses
// for real (backend `set_sleep`); the demo backend has no timer, so the UI
// pauses itself there.
import { app } from "../state.svelte";
import { backend } from "../api/backend";
import { t, tf } from "../i18n.svelte";

/** Lengths offered in the menu, in minutes. */
export const SLEEP_MINUTES = [15, 30, 45, 60, 90];

let timer = 0, watch = 0;

/** "15 min", "1 h", "1 h 30 min". */
export function sleepLabel(mins: number): string {
  const h = Math.floor(mins / 60), m = mins % 60;
  return h === 0 ? `${m} min` : m === 0 ? `${h} h` : `${h} h ${m} min`;
}

/** Countdown text: "12:05", "1:02:05". */
export function sleepCountdown(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const p = (n: number) => String(n).padStart(2, "0");
  return s >= 3600 ? `${Math.floor(s / 3600)}:${p(Math.floor(s / 60) % 60)}:${p(s % 60)}` : `${Math.floor(s / 60)}:${p(s % 60)}`;
}

/** Rust says the timer ran out (the real backends pause by themselves). */
export function sleepFinished() { done(); }

function done() {
  clearTimeout(timer); clearInterval(watch);
  app.sleep = null;
  if (backend().kind !== "tauri" && app.playing) app.togglePlay();
  app.toastInfo(t("sleep.toast.done"));
}

/** `mins` minutes, "song" (end of this song) or null (off). */
export function setSleep(s: number | "song" | null) {
  clearTimeout(timer); clearInterval(watch);
  if (s === null) {
    app.sleep = null;
    app.toastInfo(t("sleep.toast.off"));
  } else if (s === "song") {
    app.sleep = { until: null, mins: null, endOfSong: true };
    app.toastInfo(t("sleep.toast.song"));
    const from = app.currentKey;
    if (backend().kind !== "tauri") watch = window.setInterval(() => { if (app.currentKey !== from) done(); }, 400);
  } else {
    app.sleep = { until: Date.now() + s * 60_000, mins: s, endOfSong: false };
    app.toastInfo(tf("sleep.toast.set", { time: sleepLabel(s) }));
    if (backend().kind !== "tauri") timer = window.setTimeout(done, s * 60_000);
  }
  backend().call("set_sleep", { mins: typeof s === "number" ? s : null, end_of_song: s === "song" });
}
