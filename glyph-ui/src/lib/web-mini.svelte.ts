// Web mini player: Document Picture-in-Picture (always-on-top frameless window
// without Chrome URL/tab bar, like Spotify) when supported, with BroadcastChannel
// popup or in-page card fallback.
import { mount, unmount } from "svelte";
import MiniPlayer from "./overlays/MiniPlayer.svelte";
import { inMiniWindow } from "./window";
import { isWeb, backend } from "./api/backend";
import { isYoutubeCover, type Track } from "./api/types";

const CHANNEL_NAME = "glyph-web-mini";

let fallback = $state(false);
export const isWebMiniFallback = () => !isWeb() || fallback;

let pipWindow: Window | null = null;
let pipMounted: Record<string, any> | null = null;
let pipObserver: MutationObserver | null = null;
let pipOpening = false;
let popupWindow: Window | null = null;
let pollTimer: number | null = null;
let mainChannel: BroadcastChannel | null = null;
let popupChannel: BroadcastChannel | null = null;

interface DocumentPictureInPictureApi {
  readonly window?: Window | null;
  requestWindow(options?: { width?: number; height?: number; disallowReturnToOpener?: boolean }): Promise<Window>;
}

function getDocumentPip(): DocumentPictureInPictureApi | null {
  if (typeof window === "undefined") return null;
  const api = (window as unknown as { documentPictureInPicture?: DocumentPictureInPictureApi }).documentPictureInPicture;
  return api && typeof api.requestWindow === "function" ? api : null;
}

function miniDimensions(app: any): { width: number; height: number } {
  const isLarge = Boolean(app.config?.player?.mini_large);
  return {
    width: isLarge ? Math.max(240, app.config?.player?.mini_card_w || 300) : 300,
    height: isLarge ? Math.max(220, app.config?.player?.mini_card_h || 384) : 52,
  };
}

function cleanupPip() {
  pipObserver?.disconnect();
  pipObserver = null;
  if (pipMounted) {
    try { void unmount(pipMounted); } catch {}
    pipMounted = null;
  }
  pipWindow = null;
}

function copyStylesToPip(pip: Window) {
  const srcRoot = document.documentElement;
  const dstRoot = pip.document.documentElement;
  const syncRootAttrs = () => {
    dstRoot.className = `${srcRoot.className} mini-window`.trim();
    const inlineStyle = srcRoot.getAttribute("style");
    if (inlineStyle) dstRoot.setAttribute("style", inlineStyle);
    else dstRoot.removeAttribute("style");
    dstRoot.style.background = "#0c0c0c";
    dstRoot.style.colorScheme = "dark";
  };
  syncRootAttrs();

  for (const node of document.querySelectorAll('link[rel="stylesheet"], style')) {
    if (node.tagName === "LINK") {
      const srcLink = node as HTMLLinkElement;
      if (!srcLink.href) continue;
      const link = pip.document.createElement("link");
      link.rel = "stylesheet";
      link.href = srcLink.href;
      pip.document.head.appendChild(link);
    } else {
      const style = pip.document.createElement("style");
      style.textContent = node.textContent ?? "";
      pip.document.head.appendChild(style);
    }
  }

  pip.document.title = "Glyph";
  pip.document.body.style.cssText = "margin:0;height:100%;background:#0c0c0c;overflow:hidden;";

  pipObserver = new MutationObserver(syncRootAttrs);
  pipObserver.observe(srcRoot, { attributes: true, attributeFilter: ["style", "class"] });
}

export function closeWebMini(app?: any) {
  pipOpening = false;
  if (pipWindow && !pipWindow.closed) {
    try { pipWindow.close(); } catch {}
  }
  cleanupPip();
  if (popupWindow && !popupWindow.closed) {
    try { popupWindow.close(); } catch {}
  }
  popupWindow = null;
  fallback = false;
  if (mainChannel) {
    mainChannel.postMessage({ type: "close" });
  }
  if (app && app.miniShown) app.miniShown = false;
}

/** Opens the floating Spotify-style Document PiP mini player synchronously from a click gesture. */
export function openWebMini(app: any) {
  if (!isWeb() || inMiniWindow()) {
    app.miniShown = true;
    return;
  }
  if (pipWindow && !pipWindow.closed) {
    app.miniShown = true;
    const { width, height } = miniDimensions(app);
    try {
      pipWindow.resizeTo(Math.round(width), Math.round(height));
      pipWindow.focus();
    } catch {}
    return;
  }

  const dpip = getDocumentPip();
  if (dpip) {
    if (pipOpening) return;
    pipOpening = true;
    fallback = false;
    app.miniShown = true;
    const { width, height } = miniDimensions(app);
    dpip.requestWindow({ width, height, disallowReturnToOpener: true }).then((pip) => {
      pipOpening = false;
      if (!app.miniShown) {
        try { pip.close(); } catch {}
        return;
      }
      if (popupWindow && !popupWindow.closed) {
        try { popupWindow.close(); } catch {}
      }
      popupWindow = null;
      cleanupPip();
      pipWindow = pip;
      copyStylesToPip(pip);
      const target = pip.document.createElement("div");
      target.id = "app";
      target.style.height = "100%";
      pip.document.body.appendChild(target);
      pipMounted = mount(MiniPlayer, { target, props: { pipWindow: pip } });
      pip.addEventListener("pagehide", () => {
        cleanupPip();
        if (app.miniShown) app.miniShown = false;
      }, { once: true });
    }).catch(() => {
      pipOpening = false;
      if (app.miniShown) openPopupOrFallback(app);
    });
    return;
  }

  app.miniShown = true;
  openPopupOrFallback(app);
}

export function toggleWebMini(app: any) {
  if (!isWeb() || inMiniWindow()) {
    app.miniShown = !app.miniShown;
    return;
  }
  if (app.miniShown || (pipWindow && !pipWindow.closed) || (popupWindow && !popupWindow.closed)) {
    closeWebMini(app);
  } else {
    openWebMini(app);
  }
}

export function getWebMiniUrl(): string {
  if (typeof location === "undefined") return "/web/#mini";
  const path = location.pathname.startsWith("/web") ? "/web/" : location.pathname;
  return `${location.origin}${path.replace(/\/+$/, "")}/#mini`;
}

function openPopupOrFallback(app: any) {
  if (popupWindow && !popupWindow.closed) {
    try {
      popupWindow.focus();
      return;
    } catch {}
  }

  const { width: w, height: h } = miniDimensions(app);
  const left = typeof window !== "undefined" ? Math.max(0, Math.round(window.screenX + (window.outerWidth - w) / 2)) : 100;
  const top = typeof window !== "undefined" ? Math.max(0, Math.round(window.screenY + (window.outerHeight - h) / 2)) : 100;
  const features = `popup=yes,width=${w},height=${h},left=${left},top=${top},resizable=yes,scrollbars=no`;

  try {
    popupWindow = window.open(getWebMiniUrl(), "glyph-mini", features);
  } catch {
    popupWindow = null;
  }

  if (!popupWindow || popupWindow.closed || typeof popupWindow.closed === "undefined") {
    fallback = true;
    popupWindow = null;
    return;
  }

  fallback = false;
  try {
    popupWindow.focus();
  } catch {}

  if (pollTimer) clearInterval(pollTimer);
  pollTimer = window.setInterval(() => {
    if (popupWindow && popupWindow.closed) {
      clearInterval(pollTimer!);
      pollTimer = null;
      popupWindow = null;
      if (app.miniShown) app.miniShown = false;
    }
  }, 400);
}

export function syncWebMiniShown(app: any, shown: boolean) {
  if (!isWeb() || inMiniWindow()) return;
  if (!shown) {
    closeWebMini();
    return;
  }

  if (pipWindow && !pipWindow.closed) {
    try { pipWindow.focus(); } catch {}
    return;
  }
  if (pipOpening || fallback) return;

  openPopupOrFallback(app);
}

export function notifyWebMiniClosed() {
  if (typeof BroadcastChannel === "undefined") return;
  try {
    const ch = popupChannel ?? new BroadcastChannel(CHANNEL_NAME);
    ch.postMessage({ type: "mini_shown", shown: false });
  } catch {}
}

export function requestWebMiniSync() {
  if (popupChannel) {
    popupChannel.postMessage({ type: "request_sync" });
  }
}

export function setupWebMiniMain(app: any): () => void {
  if (!isWeb() || inMiniWindow() || typeof BroadcastChannel === "undefined") {
    return () => {};
  }
  if (mainChannel) mainChannel.close();
  const ch = new BroadcastChannel(CHANNEL_NAME);
  mainChannel = ch;

  const sendSync = () => {
    if (!app.ready) {
      setTimeout(sendSync, 100);
      return;
    }
    try {
      ch.postMessage({
        type: "sync",
        player: JSON.parse(JSON.stringify(app.player)),
        queue: JSON.parse(JSON.stringify(app.queue)),
        library: JSON.parse(JSON.stringify(app.library)),
        config: JSON.parse(JSON.stringify(app.config)),
        position: app.position,
        playing: app.playing,
      });
    } catch {}
  };

  ch.onmessage = (e) => {
    const data = e.data;
    if (!data || typeof data !== "object") return;
    switch (data.type) {
      case "request_sync":
        sendSync();
        break;
      case "cmd": {
        const { cmd, args } = data;
        if (cmd === "toggle_play") void app.togglePlay();
        else if (cmd === "next") void app.next();
        else if (cmd === "previous") void app.previous();
        else if (cmd === "seek") void app.seek(args?.position ?? 0);
        else if (cmd === "set_volume") void app.setVolume(args?.volume ?? 1);
        else if (cmd === "set_shuffle") void app.setShuffle(Boolean(args?.on));
        else if (cmd === "set_repeat") void backend().call("set_repeat", args).catch(() => {});
        else if (cmd === "like") void backend().call("like", args).catch(() => {});
        else if (cmd === "set_config") {
          if (args?.config) app.setConfig(() => args.config);
          else backend().call("set_config", args).catch(() => {});
        } else {
          backend().call(cmd, args).catch(() => {});
        }
        break;
      }
      case "cover": {
        const { id, track } = data;
        backend().cover(track).then((url) => {
          ch.postMessage({ type: "cover_res", id, url });
        }).catch(() => {
          ch.postMessage({ type: "cover_res", id, url: null });
        });
        break;
      }
      case "mini_shown": {
        if (!data.shown) {
          app.miniShown = false;
          if (popupWindow && !popupWindow.closed) {
            try { popupWindow.close(); } catch {}
          }
          popupWindow = null;
        }
        break;
      }
    }
  };

  let offPlayer: (() => void) | undefined;
  let offQueue: (() => void) | undefined;
  let offLibrary: (() => void) | undefined;
  let offConfig: (() => void) | undefined;
  let dead = false;

  const hookSubscriptions = () => {
    if (dead) return;
    try {
      const b = backend();
      offPlayer = b.on("player", (p) => {
        try { ch.postMessage({ type: "state_update", player: JSON.parse(JSON.stringify(p)) }); } catch {}
      });
      offQueue = b.on("queue", (q) => {
        try { ch.postMessage({ type: "state_update", queue: JSON.parse(JSON.stringify(q)) }); } catch {}
      });
      offLibrary = b.on("library", (l) => {
        try { ch.postMessage({ type: "state_update", library: JSON.parse(JSON.stringify(l)) }); } catch {}
      });
      offConfig = b.on("config", (c) => {
        try { ch.postMessage({ type: "state_update", config: JSON.parse(JSON.stringify(c)) }); } catch {}
      });
    } catch {
      setTimeout(hookSubscriptions, 100);
    }
  };
  hookSubscriptions();

  // Position streaming while playing:
  const posInterval = window.setInterval(() => {
    if (app.playing && popupWindow && !popupWindow.closed) {
      try {
        ch.postMessage({
          type: "pos_tick",
          position: app.position,
        });
      } catch {}
    }
  }, 200);

  return () => {
    dead = true;
    clearInterval(posInterval);
    if (pollTimer) clearInterval(pollTimer);
    offPlayer?.();
    offQueue?.();
    offLibrary?.();
    offConfig?.();
    ch.close();
    if (mainChannel === ch) mainChannel = null;
  };
}

export function setupWebMiniPopup(app: any): () => void {
  if (!isWeb() || !inMiniWindow() || typeof BroadcastChannel === "undefined") {
    return () => {};
  }
  if (popupChannel) popupChannel.close();
  const ch = new BroadcastChannel(CHANNEL_NAME);
  popupChannel = ch;

  let hooked = false;
  let dead = false;

  const hookBackend = () => {
    if (hooked || dead) return;
    try {
      const b = backend();
      if (!b) return;
      hooked = true;
      b.call = (cmd: string, args?: Record<string, unknown>) => {
        ch.postMessage({ type: "cmd", cmd, args });
        return Promise.resolve() as any;
      };
      b.cover = (track: Track) => {
        if (track?.cover_url && !isYoutubeCover(track.cover_url)) return Promise.resolve(track.cover_url);
        return new Promise<string | null>((resolve) => {
          const id = Math.random().toString(36).slice(2);
          const onMsg = (e: MessageEvent) => {
            if (e.data?.type === "cover_res" && e.data?.id === id) {
              ch.removeEventListener("message", onMsg);
              resolve(e.data.url ?? null);
            }
          };
          ch.addEventListener("message", onMsg);
          ch.postMessage({ type: "cover", id, track });
          setTimeout(() => {
            ch.removeEventListener("message", onMsg);
            resolve(null);
          }, 4000);
        });
      };
    } catch {
      setTimeout(hookBackend, 50);
    }
  };
  hookBackend();

  ch.onmessage = (e) => {
    const data = e.data;
    if (!data || typeof data !== "object") return;
    switch (data.type) {
      case "sync":
        if (data.config) app.config = data.config;
        if (data.library) app.library = data.library;
        if (data.queue) app.queue = data.queue;
        if (data.player) {
          app.player = data.player;
          app.posAnchor = { pos: data.position ?? data.player.position ?? 0, at: performance.now() };
        }
        app.ready = true;
        hookBackend();
        break;
      case "state_update":
        if (data.config) app.config = data.config;
        if (data.library) app.library = data.library;
        if (data.queue) app.queue = data.queue;
        if (data.player) {
          app.player = { ...app.player, ...data.player };
          if (data.player.position != null) {
            app.posAnchor = { pos: data.player.position, at: performance.now() };
          }
        }
        break;
      case "pos_tick":
        if (app.player && data.position != null) {
          app.player.position = data.position;
          app.posAnchor = { pos: data.position, at: performance.now() };
        }
        break;
      case "close":
        window.close();
        break;
    }
  };

  ch.postMessage({ type: "request_sync" });

  const onUnload = () => {
    try {
      ch.postMessage({ type: "mini_shown", shown: false });
    } catch {}
  };
  window.addEventListener("beforeunload", onUnload);

  return () => {
    dead = true;
    window.removeEventListener("beforeunload", onUnload);
    ch.close();
    if (popupChannel === ch) popupChannel = null;
  };
}
