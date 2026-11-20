// State and actions of Settings > Account (ui/account.rs AccountUi + the
// Glyph account_* methods). Optional sign-in (passkey in the browser or a
// pairing code), devices with the server's device limit, library sync.
// Without an account nothing here runs. On the official server the browser
// page is the default sign-in; on any other server sign-in stays in the app
// (its own page only opens after a warning, it could read the password).
import { backend } from "../api/backend";
import { app } from "../state.svelte";
import { t, tf } from "../i18n.svelte";
import { errText, openUrl, saveText } from "./util";
import type { AccountStatus, LoginPoll, LoginStart } from "./types";

type Login =
  | { kind: "idle" }
  | { kind: "starting" }
  | { kind: "waiting"; start: LoginStart; full: boolean }
  | { kind: "failed"; text: string };

export const ac = $state({
  st: null as AccountStatus | null,
  login: { kind: "idle" } as Login,
  /** The running sign-in shows only a pairing code (no browser page). */
  pairingOnly: false,
  /** Feedback line under the actions (approve, rename, ...). */
  notice: null as { text: string; ok: boolean } | null,
  recovery: null as string | null,
  recoveryVisible: false,
  /** Signed-out form: "create account" instead of "sign in". */
  register: false,
  /** On the public server the browser page is the default; this shows the in-app form instead. */
  inApp: false,
  /** A password call is running (key derivation + request). */
  busy: false,
  /** Profile avatar upload/crop dialog is running. */
  avatarBusy: false,
  /** Not the official server: the warning before its own sign-in page. */
  webWarning: false,
});

const CODES: Record<string, string> = {
  login_failed: "account.err.login", password_too_short: "account.err.pw_short", password_too_long: "account.err.pw_long",
  wrong_password: "account.err.wrong_password", no_password: "account.library.escrow.nopw_desc", bad_salt: "privacy.event.bad_salt",
  signed_out: "account.signed_out", device_limit: "account.err.device_limit", handle_taken: "account.err.taken",
  handle_invalid: "account.err.invalid", handle_reserved: "account.err.reserved", handle_cooldown: "account.err.cooldown",
  too_many_devices: "account.err.too_many", not_found: "account.err.code", rate_limited: "account.err.slow",
  account_full: "account.storage.full",
};
/** account.rs err_text: the backend rejects with a short code. */
export function errMessage(e: unknown): string {
  const m = errText(e);
  if (m.includes("account_full")) return t("account.storage.full");
  return t(CODES[m] ?? "account.err.generic");
}

export async function loadStatus() {
  const s = await backend().call<AccountStatus>("account_status");
  if (s) ac.st = s;
  return s;
}

/** Called when the Account page opens: availability, device list. */
export function pageOpened() {
  ac.notice = null;
  app.ensureOwnProfile();
  return loadStatus();
}

/** Runs an account call and shows its outcome as a toast when signed in. */
async function call<T>(cmd: string, args: Record<string, unknown>, done: (r: T) => void) {
  try { done(await backend().call<T>(cmd, args)); }
  catch (e) {
    if (errText(e) === "signed_out") await loadStatus();
    if (ac.st?.signed_in) app.toastInfo(errMessage(e));
    else ac.notice = { text: errMessage(e), ok: false };
  }
  loadStatus();
}

export function setMode(register: boolean) { ac.register = register; ac.notice = null; ac.login = { kind: "idle" }; }

let loginSeq = 0;
/** Sign in with the server's browser page (passkey, password there) or a pairing code. */
export async function signIn(browser: boolean) {
  const seq = ++loginSeq;
  ac.login = { kind: "starting" };
  ac.pairingOnly = !browser;
  ac.notice = null;
  let start: LoginStart;
  try { start = (await backend().call<LoginStart>("account_login_start"))!; }
  catch (e) { ac.login = { kind: "failed", text: errMessage(e) }; return; }
  ac.login = { kind: "waiting", start, full: false };
  if (browser) openUrl(start.url);
  const deadline = Date.now() + start.expires_in_s * 1000;
  while (seq === loginSeq) {
    await new Promise((r) => setTimeout(r, 2000));
    if (seq !== loginSeq) return;
    if (Date.now() > deadline) { ac.login = { kind: "failed", text: t("account.login.expired") }; return; }
    let r: LoginPoll | undefined;
    try { r = await backend().call<LoginPoll>("account_login_poll", { login_id: start.login_id }); }
    catch (e: any) {
      const retry = typeof e?.retryAfter === "number" ? e.retryAfter : 0;
      if (e?.status === 429 || errText(e) === "rate_limited" || retry > 0) {
        await new Promise((res) => setTimeout(res, Math.min(60, Math.max(5, retry || 10)) * 1000));
      }
      continue;
    }
    if (!r) continue;
    if (r.state === "device_limit") ac.login = { kind: "waiting", start, full: true };
    else if (r.state === "expired") { ac.login = { kind: "failed", text: t("account.login.expired") }; return; }
    else if (r.state === "done") {
      ac.login = { kind: "idle" }; ac.pairingOnly = false;
      app.toast(tf("account.toast.signed_in", { handle: r.handle }));
      loadStatus();
      return;
    }
  }
}

export function cancelLogin() { loginSeq++; ac.login = { kind: "idle" }; ac.pairingOnly = false; }

export function webConfirmed() {
  ac.webWarning = false;
  backend().call("privacy_record_web_login");
  signIn(true);
}

/** Sign in or create an account with handle + password (form on the page). */
export async function submitPassword(handle: string, pw: string, pw2: string): Promise<boolean> {
  if (ac.busy) return false;
  handle = handle.trim();
  if (!handle || !pw) { ac.login = { kind: "failed", text: t("account.err.fill") }; return false; }
  if (ac.register && pw !== pw2) { ac.login = { kind: "failed", text: t("account.err.pw_mismatch") }; return false; }
  ac.busy = true; ac.login = { kind: "idle" };
  try {
    const r = await backend().call<{ recovery_code?: string | null; handle: string }>("account_password_submit", { handle, password: pw, register: ac.register });
    ac.recovery = r?.recovery_code ?? null; ac.recoveryVisible = false;
    app.toast(tf("account.toast.signed_in", { handle: r?.handle ?? handle }));
    await loadStatus();
    ac.busy = false;
    return true;
  } catch (e) { ac.login = { kind: "failed", text: errMessage(e) }; }
  ac.busy = false;
  return false;
}

export function unlockKeys(pw: string, done: () => void) {
  if (!pw) return;
  call("account_unlock_keys", { password: pw }, () => { app.toast(t("account.library.unlocked")); done(); });
}

export function changePassword(pw: string, pw2: string, done: () => void) {
  if (pw !== pw2) { app.toastInfo(t("account.err.pw_mismatch")); return; }
  call("account_change_password", { password: pw }, () => { done(); app.toast(t("account.password.saved")); });
}

export function openSecurity(action: "password" | "passkey" | "recovery" | "delete") {
  call<{ url: string }>("account_link_passkey", {}, (r) => {
    if (!r?.url) return;
    const sep = r.url.includes("?") ? "&" : "?";
    openUrl(`${r.url}${sep}action=${encodeURIComponent(action)}`);
    app.toast(t(action === "passkey" ? "account.passkey.opened" : "account.security.opened"));
  });
}

export function linkPasskey() { openSecurity("passkey"); }

export function removeDevice(id: string) {
  call("account_remove_device", { id }, () => { app.toast(t("account.device.removed")); });
}

export function rename(handle: string, done: () => void) {
  const h = handle.trim();
  if (!h) return;
  call<{ handle: string }>("account_rename", { handle: h }, (a) => { app.toast(tf("account.handle.saved", { handle: a.handle })); done(); });
}

export function approve(code: string, done: () => void) {
  const c = code.trim();
  if (!c) return;
  call<{ device_name: string; device_limit_reached: boolean }>("account_approve", { code: c }, (r) => {
    if (r.device_limit_reached) app.toastInfo(tf("account.pair.full", { name: r.device_name }));
    else app.toast(tf("account.pair.ok", { name: r.device_name }));
    done();
  });
}

export function newRecoveryCode() {
  call<string>("account_recovery", {}, (code) => { ac.recovery = code; ac.recoveryVisible = false; });
}

export async function copyRecovery(copy: (s: string) => Promise<boolean>) {
  if (ac.recovery && (await copy(ac.recovery))) app.copied(t("account.recovery.copied"));
}

export function downloadRecovery() {
  if (!ac.recovery) return;
  const handle = ac.st?.handle ?? "";
  try { saveText(`glyph-recovery-code-${handle}.txt`, tf("account.recovery.file", { handle, code: ac.recovery })); app.toast(t("account.recovery.saved")); }
  catch (e) { app.toastInfo(tf("sp.transfer.error", { e: errText(e) })); }
}

export function signOut() {
  call("account_sign_out", {}, () => { ac.recovery = null; ac.notice = null; app.toast(t("account.toast.signed_out")); });
}

export function signOutAll() {
  call("account_sign_out_all", {}, () => { ac.recovery = null; ac.notice = null; app.toast(t("account.toast.signed_out_all")); });
}

function askSignOut(all: boolean) {
  const handle = ac.st?.handle || app.account?.handle || "";
  const at = handle ? `@${handle}` : "";
  app.ask({
    title: t(all ? "account.devices.signout_all.confirm.title" : "account.signout.confirm.title"),
    body: tf(all ? "account.devices.signout_all.confirm.body" : "account.signout.confirm.body", { handle: at }),
    ok: t(all ? "account.devices.signout_all" : "account.signout"),
    danger: true,
    run: all ? signOutAll : signOut,
  });
}

export function requestSignOut() {
  askSignOut(false);
}

export function requestSignOutAll() {
  askSignOut(true);
}

export function deleteAccount(typed: string) {
  const h = typed.trim().replace(/^@/, "").toLowerCase();
  if (h !== ac.st?.handle) { ac.notice = { text: t("account.delete.mismatch"), ok: false }; return; }
  call("account_delete", { handle: h }, () => { ac.recovery = null; app.toast(t("account.toast.deleted")); });
}

export function syncNow() { call("account_sync_now", {}, () => {}); }
export function chooseMerge(yes: boolean) { call("account_merge", { yes }, () => {}); }

export async function pickAvatar() {
  if (ac.avatarBusy) return;
  ac.avatarBusy = true;
  try {
    await backend().call("profile_pick_image", { banner: false });
    await app.ensureOwnProfile(true);
    loadStatus();
  } catch (e) {
    app.toastInfo(errMessage(e));
  }
  ac.avatarBusy = false;
}
