// Web App (PWA) install & mobile standalone helpers: registers the service
// worker on `/web/`, captures Chromium's native `beforeinstallprompt`, detects
// iOS / iPadOS "Add to Home Screen" & standalone mode, and syncs the virtual
// keyboard inset (`--ime-bottom`) from `visualViewport` to match Android.
import { isTauri, isWeb } from "./api/backend";
import { load, save } from "./api/web-store";

interface BeforeInstallPromptEvent extends Event {
  readonly platforms: string[];
  readonly userChoice: Promise<{ outcome: "accepted" | "dismissed"; platform: string }>;
  prompt(): Promise<void>;
}

const DISMISS_KEY = "install_dismissed_at";
const DISMISS_TTL_MS = 14 * 24 * 3600_000;

function detectStandalone(): boolean {
  if (typeof window === "undefined") return false;
  const nav = navigator as Navigator & { standalone?: boolean };
  if (nav.standalone === true) return true;
  try {
    return (
      window.matchMedia("(display-mode: standalone)").matches
      || window.matchMedia("(display-mode: minimal-ui)").matches
    );
  } catch {
    return false;
  }
}

function detectPlatform() {
  if (typeof navigator === "undefined") {
    return { ios: false, ipad: false, iosSafari: false, macSafari: false, android: false, mobile: false };
  }
  const ua = navigator.userAgent || "";
  const plat = navigator.platform || "";
  const touch = navigator.maxTouchPoints || 0;
  const ipad = /iPad/i.test(ua) || (plat === "MacIntel" && touch > 1);
  const iphone = /iPhone|iPod/i.test(ua);
  const ios = iphone || ipad;
  const iosSafari = ios && /Safari/i.test(ua) && !/CriOS|FxiOS|EdgiOS|OPiOS|DuckDuckGo|Instagram|FBAN|FBAV|Line\//i.test(ua);
  const macSafari = !ios && /Macintosh|Mac OS X/i.test(ua) && /Safari/i.test(ua) && !/Chrome|Chromium|Edg|OPR|Firefox/i.test(ua);
  const android = /Android/i.test(ua);
  const mobile = ios || android || /Mobile/i.test(ua);
  return { ios, ipad, iosSafari, macSafari, android, mobile };
}

function isDismissedRecently(): boolean {
  const at = load<number>(DISMISS_KEY);
  return typeof at === "number" && Number.isFinite(at) && Date.now() - at < DISMISS_TTL_MS;
}

const initialPlat = detectPlatform();
const initialStandalone = detectStandalone();

let deferredPrompt: BeforeInstallPromptEvent | null = null;

export const pwa = $state({
  standalone: initialStandalone,
  installed: initialStandalone,
  canPrompt: false,
  sheetOpen: false,
  bannerDismissed: isDismissedRecently(),
  ios: initialPlat.ios,
  ipad: initialPlat.ipad,
  iosSafari: initialPlat.iosSafari,
  macSafari: initialPlat.macSafari,
  android: initialPlat.android,
  mobile: initialPlat.mobile,
});

export function openInstallSheet() {
  pwa.sheetOpen = true;
}

export function closeInstallSheet() {
  pwa.sheetOpen = false;
}

export function dismissInstallBanner() {
  pwa.bannerDismissed = true;
  save(DISMISS_KEY, Date.now());
}

/** Triggers Chromium's native install prompt when available. */
export async function triggerNativeInstall(): Promise<"accepted" | "dismissed" | "unavailable"> {
  if (!deferredPrompt) return "unavailable";
  const ev = deferredPrompt;
  deferredPrompt = null;
  pwa.canPrompt = false;
  try {
    await ev.prompt();
    const res = await ev.userChoice;
    if (res.outcome === "accepted") {
      pwa.installed = true;
      pwa.sheetOpen = false;
    }
    return res.outcome;
  } catch {
    return "dismissed";
  }
}

/**
 * Starts PWA listeners, registers `/web/sw.js`, and mirrors mobile virtual
 * keyboard height into `--ime-bottom` (like Android's `MainActivity`).
 */
export function installPwa(): () => void {
  if (typeof window === "undefined" || isTauri()) return () => {};

  const syncMode = () => {
    const standalone = detectStandalone();
    pwa.standalone = standalone;
    if (standalone) pwa.installed = true;
    document.documentElement.classList.toggle("standalone", standalone);
    document.documentElement.classList.toggle("ios", pwa.ios);
  };
  syncMode();

  const mqStandalone = window.matchMedia?.("(display-mode: standalone)");
  mqStandalone?.addEventListener?.("change", syncMode);

  const onBeforeInstall = (e: Event) => {
    e.preventDefault();
    deferredPrompt = e as BeforeInstallPromptEvent;
    pwa.canPrompt = true;
  };
  const onAppInstalled = () => {
    deferredPrompt = null;
    pwa.canPrompt = false;
    pwa.installed = true;
    pwa.sheetOpen = false;
  };
  window.addEventListener("beforeinstallprompt", onBeforeInstall);
  window.addEventListener("appinstalled", onAppInstalled);

  // Register the service worker on the built/served web player (`/web/`).
  if (isWeb() && "serviceWorker" in navigator) {
    const swUrl = `${import.meta.env.BASE_URL}sw.js`;
    const scope = import.meta.env.BASE_URL;
    navigator.serviceWorker.register(swUrl, { scope }).catch(() => { /* private window or dev */ });
  }

  // Deep-link `/web/?install=1` (from glyph.my/download on iPhone/iPad) opens the install sheet.
  if (isWeb() && !pwa.installed) {
    try {
      const sp = new URLSearchParams(window.location.search);
      if (sp.get("install") === "1") {
        pwa.sheetOpen = true;
        sp.delete("install");
        const qs = sp.toString();
        const nextUrl = window.location.pathname + (qs ? `?${qs}` : "") + window.location.hash;
        window.history.replaceState(window.history.state, "", nextUrl);
      }
    } catch { /* ignore */ }
  }

  // Sync mobile soft keyboard height into `--ime-bottom` just like Android's native insets.
  const vv = window.visualViewport;
  const syncIme = () => {
    if (!vv) return;
    const keyboard = Math.max(0, Math.round(window.innerHeight - vv.height - vv.offsetTop));
    // Ignore tiny browser toolbar resizes (< 90px); only treat real software keyboards as IME.
    const ime = keyboard > 90 ? keyboard : 0;
    document.documentElement.style.setProperty("--ime-bottom", `${ime}px`);
  };
  if (vv) {
    syncIme();
    vv.addEventListener("resize", syncIme);
    vv.addEventListener("scroll", syncIme);
  }

  return () => {
    mqStandalone?.removeEventListener?.("change", syncMode);
    window.removeEventListener("beforeinstallprompt", onBeforeInstall);
    window.removeEventListener("appinstalled", onAppInstalled);
    if (vv) {
      vv.removeEventListener("resize", syncIme);
      vv.removeEventListener("scroll", syncIme);
    }
  };
}
