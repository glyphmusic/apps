// Helpers of the Connect panel and the status strip (ui/connect.rs kind_label,
// platform_icon, playing_on, this_device, this_icon). `app.devices` (types.Device)
// carries what the backend knows about each device; the optional fields below
// are what the backend adds for the panel (see api/demo/extra.ts `devices_rescan`).
import type { Device } from "./api/types";
import { app } from "./state.svelte";
import { t } from "./i18n.svelte";
import { pwa } from "./pwa.svelte";

export type DeviceX = Device & {
  /** Glyph apps: the platform it runs on ("windows", "android", ...). */
  platform?: string;
  /** Network devices: "Google Cast", "Sonos", "DLNA", "MPD", "AirPlay". */
  protocol?: string;
  /** Glyph apps: what plays there ("Artist · Title") or null. */
  track?: string | null;
  /** Glyph apps: it plays right now / it is busy in a Jam or controls another device. */
  playing?: boolean;
  busy?: boolean;
  /** Connecting now / could not be reached / last problem. */
  connecting?: boolean;
  unreachable?: boolean;
  error?: string | null;
  /** Icon for network devices (tv, speaker, radio, ...). */
  icon?: string;
};

/** Icon of a signed-in Glyph app by the platform it runs on. */
export const platformIcon = (platform?: string) => platformIconFor(platform);

export function platformIconFor(platform?: string, name?: string): string {
  const p = (platform ?? "").toLowerCase();
  const n = (name ?? "").toLowerCase();
  // Glyph Web reports the OS it runs on, so its name tells it apart.
  if (p === "web" || p === "browser" || n.startsWith("glyph web")) return "globe";
  if (p === "android" || p === "ios") return "smartphone";
  if (p === "macos" || n.includes("macbook") || n.includes("laptop") || n.includes("thinkpad")) return "laptop";
  return "monitor";
}

/** Protocol name under a network device. */
export function kindLabel(d: DeviceX): string {
  if (d.protocol) return d.protocol;
  return { cast: "Google Cast", airplay: "AirPlay", upnp: "DLNA", glyph: "Glyph", account: "Glyph", this: "" }[d.kind];
}

export function deviceIcon(d: DeviceX): string {
  if (d.icon) return d.icon;
  switch (d.kind) {
    case "this": return thisIcon();
    case "glyph": case "account": return platformIconFor(d.platform, d.name);
    case "cast": return "cast";
    case "airplay": return "airplay";
    default: return "speaker";
  }
}

const mobilePhoneWeb = () => app.platform === "web" && (pwa.mobile || app.compact);
export const thisIcon = () => (
  app.platform === "android" || app.platform === "ios" || mobilePhoneWeb()
    ? "smartphone"
    : app.platform === "web"
      ? (pwa.standalone ? "monitor" : "globe")
      : "monitor"
);
export const thisName = () => t(
  app.platform === "android" || app.platform === "ios" || (mobilePhoneWeb() && pwa.standalone)
    ? "connect.this_phone"
    : app.platform === "web"
      ? (pwa.standalone ? "connect.this_computer" : "connect.this_browser")
      : "connect.this_computer",
);
export const isGlyphApp = (d: DeviceX) => d.kind === "glyph" || d.kind === "account";

/** The other device music plays on (icon, name), if any. */
export function playingOn(devices: DeviceX[]): { icon: string; name: string } | null {
  const d = devices.find((x) => x.active && x.kind !== "this");
  return d ? { icon: deviceIcon(d), name: d.name } : null;
}

/** Another Glyph app plays while this one does not: it can be taken over with one click. */
export function connectTo(devices: DeviceX[]): DeviceX | null {
  return devices.find((d) => isGlyphApp(d) && !d.active && !d.offline && !d.busy && d.playing && d.track) ?? null;
}
