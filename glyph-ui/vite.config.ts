import { defineConfig, type Plugin } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { fileURLToPath } from "node:url";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// Icons, fonts and UI strings stay in glyph-apps/assets (one copy for the
// whole repo); the UI imports them from there.
const assets = fileURLToPath(new URL("../assets", import.meta.url));
const version = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8")).version as string;

const WEB_MANIFEST = JSON.stringify({
  id: "/web/",
  name: "Glyph",
  short_name: "Glyph",
  description: "Every song, one quiet player.",
  start_url: "/web/",
  scope: "/web/",
  display: "standalone",
  display_override: ["standalone", "minimal-ui"],
  background_color: "#030303",
  theme_color: "#030303",
  categories: ["music", "entertainment"],
  icons: [
    { src: "/web/icon-64.png", sizes: "64x64", type: "image/png", purpose: "any" },
    { src: "/web/icon-192.png", sizes: "192x192", type: "image/png", purpose: "any" },
    { src: "/web/icon-256.png", sizes: "256x256", type: "image/png", purpose: "any" },
    { src: "/web/icon-512.png", sizes: "512x512", type: "image/png", purpose: "any" },
    { src: "/web/icon-maskable-512.png", sizes: "512x512", type: "image/png", purpose: "maskable" },
  ],
  shortcuts: [
    { name: "Search", url: "/web/search", icons: [{ src: "/web/icon-192.png", sizes: "192x192", type: "image/png" }] },
    { name: "Library", url: "/web/library", icons: [{ src: "/web/icon-192.png", sizes: "192x192", type: "image/png" }] },
    { name: "Liked Songs", url: "/web/liked", icons: [{ src: "/web/icon-192.png", sizes: "192x192", type: "image/png" }] },
  ],
}, null, 2);

const PWA_ICONS = [
  "apple-touch-icon.png",
  "icon-192.png",
  "icon-256.png",
  "icon-512.png",
  "icon-maskable-512.png",
] as const;

function buildServiceWorker(cacheId: string): string {
  return `// Glyph Web App service worker: caches the app shell and immutable hashed
// bundles so Glyph launches fast from the home screen and works cleanly as an
// installable PWA. API requests (/v1/*) and cross-origin audio streams are
// never intercepted.
const CACHE = ${JSON.stringify(cacheId)};
const SHELL = [
  "/web/",
  "/web/index.html",
  "/web/manifest.webmanifest",
  "/web/icon-64.png",
  "/web/icon-192.png",
  "/web/icon-256.png",
  "/web/apple-touch-icon.png",
];

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches.open(CACHE)
      .then((cache) => cache.addAll(SHELL).catch(() => {}))
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches.keys()
      .then((keys) => Promise.all(keys.filter((k) => k.startsWith("glyph-web-") && k !== CACHE).map((k) => caches.delete(k))))
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;
  if (!url.pathname.startsWith("/web/")) return;
  if (url.pathname === "/web/sw.js") return;

  // Navigation requests under /web/*: network-first so updates land immediately,
  // falling back to the cached SPA shell if offline.
  if (req.mode === "navigate") {
    event.respondWith(
      fetch(req)
        .then((res) => {
          if (res.ok) {
            const copy = res.clone();
            caches.open(CACHE).then((c) => c.put("/web/index.html", copy)).catch(() => {});
          }
          return res;
        })
        .catch(() => caches.match("/web/index.html").then((hit) => hit || caches.match("/web/"))),
    );
    return;
  }

  // Hashed bundles (/web/assets/*): immutable cache-first for instant app launches.
  if (url.pathname.startsWith("/web/assets/")) {
    event.respondWith(
      caches.match(req).then((hit) => {
        if (hit) return hit;
        return fetch(req).then((res) => {
          if (res.ok) {
            const copy = res.clone();
            caches.open(CACHE).then((c) => c.put(req, copy)).catch(() => {});
          }
          return res;
        });
      }),
    );
    return;
  }

  // Static app icons and manifest: stale-while-revalidate.
  event.respondWith(
    caches.match(req).then((hit) => {
      const fresh = fetch(req).then((res) => {
        if (res.ok) {
          const copy = res.clone();
          caches.open(CACHE).then((c) => c.put(req, copy)).catch(() => {});
        }
        return res;
      }).catch(() => hit);
      return hit || fresh;
    }),
  );
});
`;
}

function webPwaPlugin(): Plugin {
  const cacheId = `glyph-web-${version}-${Date.now().toString(36)}`;
  const swSource = buildServiceWorker(cacheId);
  return {
    name: "glyph-web-pwa",
    transformIndexHtml() {
      return [
        { tag: "link", attrs: { rel: "manifest", href: "/web/manifest.webmanifest" }, injectTo: "head" },
        { tag: "link", attrs: { rel: "apple-touch-icon", sizes: "180x180", href: "/web/apple-touch-icon.png" }, injectTo: "head" },
        { tag: "meta", attrs: { name: "theme-color", content: "#030303" }, injectTo: "head" },
        { tag: "meta", attrs: { name: "mobile-web-app-capable", content: "yes" }, injectTo: "head" },
        { tag: "meta", attrs: { name: "apple-mobile-web-app-capable", content: "yes" }, injectTo: "head" },
        { tag: "meta", attrs: { name: "apple-mobile-web-app-title", content: "Glyph" }, injectTo: "head" },
        { tag: "meta", attrs: { name: "apple-mobile-web-app-status-bar-style", content: "black-translucent" }, injectTo: "head" },
        { tag: "meta", attrs: { name: "format-detection", content: "telephone=no" }, injectTo: "head" },
      ];
    },
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        if (req.url === "/web/manifest.webmanifest") {
          res.setHeader("content-type", "application/manifest+json; charset=utf-8");
          res.end(WEB_MANIFEST);
          return;
        }
        if (req.url === "/web/sw.js") {
          res.setHeader("content-type", "text/javascript; charset=utf-8");
          res.setHeader("cache-control", "no-cache");
          res.end(swSource);
          return;
        }
        for (const file of PWA_ICONS) {
          if (req.url === `/web/${file}`) {
            res.setHeader("content-type", "image/png");
            res.end(readFileSync(join(assets, "icons", file)));
            return;
          }
        }
        next();
      });
    },
    generateBundle() {
      this.emitFile({ type: "asset", fileName: "manifest.webmanifest", source: WEB_MANIFEST });
      this.emitFile({ type: "asset", fileName: "sw.js", source: swSource });
      for (const file of PWA_ICONS) {
        this.emitFile({
          type: "asset",
          fileName: file,
          source: readFileSync(join(assets, "icons", file)),
        });
      }
    },
  };
}

// `--mode web` (npm run build:web / dev:web): the web player for glyph.my/web
// (docs/WEB.md). Same UI, served under /web/, talking to glyph-server.
// Locally `dev:web` forwards /v1 to GLYPH_SERVER (default https://glyph.my);
// audio still comes straight from the providers.
export default defineConfig(({ mode }) => {
  const web = mode === "web";
  const server = process.env.GLYPH_SERVER || "https://glyph.my";
  return {
    plugins: web ? [svelte(), webPwaPlugin()] : [svelte()],
    base: web ? "/web/" : "/",
    clearScreen: false,
    resolve: { alias: { $assets: assets, $lib: fileURLToPath(new URL("./src/lib", import.meta.url)) } },
    server: web
      ? { port: 1421, strictPort: true, fs: { allow: [".", assets] }, proxy: { "/v1": { target: server, changeOrigin: true } } }
      : { port: 1420, strictPort: true, fs: { allow: [".", assets] } },
    envPrefix: ["VITE_", "TAURI_ENV_"],
    define: {
      "import.meta.env.VITE_GLYPH_VERSION": JSON.stringify(version),
      "import.meta.env.VITE_GLYPH_WEB": JSON.stringify(web ? "1" : ""),
    },
    build: {
      target: "es2022",
      outDir: web ? "dist-web" : "dist",
      emptyOutDir: true,
      rolldownOptions: {
        output: {
          codeSplitting: {
            groups: [
              { name: "vendor", test: /[\\/]node_modules[\\/]/, priority: 20 },
              { name: "pages", test: /[\\/]src[\\/]lib[\\/]pages[\\/]/, maxSize: 380_000, priority: 18 },
              { name: "settings", test: /[\\/]src[\\/]lib[\\/]settings[\\/]/, maxSize: 380_000, priority: 15 },
              { name: "compact", test: /[\\/]src[\\/]lib[\\/]compact[\\/]/, priority: 10 },
            ],
          },
        },
      },
    },
  };
});
