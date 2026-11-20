// Whether the signed-in account may open the server dashboard. Asked once per
// sign-in (`GET /admin/v1/me`); a "not admin" answer hides the sidebar entry.
import { backend } from "../api/backend";
import type { AdminMe } from "./api";

/** The dashboard's sub pages ("" = overview): /server/people and so on. */
export type AdminSub = "" | "people" | "settings" | "versions";
export const ADMIN_SUBS: AdminSub[] = ["people", "settings", "versions"];

class AdminState {
  me = $state<AdminMe | null>(null);
  /** Still asking the server. */
  pending = $state(false);
  /** Open sub page; back returns to the overview first. */
  sub = $state<AdminSub>("");
  /** Bumped by the refresh buttons: the page loads again. */
  reload = $state(0);
  /** Handle the answer belongs to ("" = signed out). */
  private asked = "";
  private busy = false;

  get on(): boolean { return !!this.me; }

  /** Call when the signed-in account may have changed. */
  check(handle: string | null | undefined) {
    const h = handle ?? "";
    if (h === this.asked || this.busy) return;
    this.asked = h;
    if (!h) { this.me = null; return; }
    this.busy = true;
    this.pending = true;
    backend()
      .call<AdminMe>("admin_call", { method: "GET", path: "me" })
      .then((m) => { if (this.asked === h) this.me = m ?? null; })
      .catch(() => { if (this.asked === h) this.me = null; })
      .finally(() => { this.busy = false; this.pending = false; });
  }

  /** The role was taken away while the dashboard was open. */
  lost() { this.me = null; }
}

export const admin = new AdminState();

/** One admin API call; errors are short codes (`not_admin`, `owner_only`, `bad_request: …`). */
export async function adminCall<T>(method: "GET" | "POST" | "PUT", path: string, body?: unknown): Promise<T> {
  try {
    return await backend().call<T>("admin_call", { method, path, body: body ?? null });
  } catch (e) {
    const code = String((e as Error)?.message ?? e);
    if (code === "not_admin" || code === "signed_out") admin.lost();
    throw new Error(code);
  }
}
