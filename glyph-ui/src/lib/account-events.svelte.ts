// Events the account / jam / sleep code in Rust sends on its own (signing in
// or out, a jam that ended, a jam link, the sleep timer that ran out).
import { backend } from "./api/backend";
import { app } from "./state.svelte";
import * as J from "./jam.svelte";
import { ac, loadStatus } from "./settings/account-state.svelte";
import { sleepFinished } from "./overlays/sleep.svelte";

/** Home and the other feeds come from the account now: load them again. */
function reloadFeeds() {
  app.home = { ...app.home, recs: [], trending: [], new_releases: [], loading: true };
  app.loadHome();
}

/** Signed out: back to Home with nothing of the account on screen (account_leave). */
function resetView() {
  app.view = "home";
  app.history = [];
  app.forwardHistory = [];
  app.openList = null;
  app.openArtist = null;
  app.openProfile = null;
  app.section = null;
  app.selectedRow = null;
  app.ctx = null;
  app.controlMenu = null;
  app.nameDialog = null;
  app.addSongs = null;
  app.confirm = null;
  app.artists = {};
  app.lyrics = { key: null, lyrics: null, loading: false };
  app.setQuery("");
}

function mergeQuestion() {
  app.openSettings("account");
  loadStatus();
}

export function installAccountEvents() {
  const api = backend();
  api.on("jam", (s) => J.apply(s as J.JamSnapshot));
  api.on("jam_open", () => { app.showDevices = true; app.showQueue = false; app.jamOpen = true; });
  api.on("account_merge", mergeQuestion);
  api.on("account_reset", (e) => {
    if (!e.signed_in) resetView();
    reloadFeeds();
    if (ac.st || app.settingsOpen) loadStatus();
    J.refresh();
  });
  api.on("sleep", () => sleepFinished());
  // A merge question or a jam that came before this listener.
  if (app.account?.merge_pending) mergeQuestion();
  J.refresh();
}
