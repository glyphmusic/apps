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
  name: "Glyph",
  short_name: "Glyph",
  description: "Every song, one quiet player.",
  start_url: "/web/",
  scope: "/web/",
  display: "standalone",
  background_color: "#0c0c0c",
  theme_color: "#0c0c0c",
  icons: [
    { src: "/web/icon-64.png", sizes: "64x64", type: "image/png" },
    { src: "/web/icon-256.png", sizes: "256x256", type: "image/png" },
    { src: "/web/icon-512.png", sizes: "512x512", type: "image/png", purpose: "any" },
  ],
}, null, 2);

function webPwaPlugin(): Plugin {
  return {
    name: "glyph-web-pwa",
    transformIndexHtml() {
      return [
        { tag: "link", attrs: { rel: "manifest", href: "/web/manifest.webmanifest" }, injectTo: "head" },
        { tag: "link", attrs: { rel: "apple-touch-icon", href: "/web/icon-256.png" }, injectTo: "head" },
        { tag: "meta", attrs: { name: "theme-color", content: "#0c0c0c" }, injectTo: "head" },
        { tag: "meta", attrs: { name: "mobile-web-app-capable", content: "yes" }, injectTo: "head" },
        { tag: "meta", attrs: { name: "apple-mobile-web-app-capable", content: "yes" }, injectTo: "head" },
        { tag: "meta", attrs: { name: "apple-mobile-web-app-status-bar-style", content: "black-translucent" }, injectTo: "head" },
      ];
    },
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        if (req.url === "/web/manifest.webmanifest") {
          res.setHeader("content-type", "application/manifest+json; charset=utf-8");
          res.end(WEB_MANIFEST);
          return;
        }
        next();
      });
    },
    generateBundle() {
      this.emitFile({ type: "asset", fileName: "manifest.webmanifest", source: WEB_MANIFEST });
      for (const file of ["icon-256.png", "icon-512.png"]) {
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
    build: { target: "es2022", outDir: web ? "dist-web" : "dist", emptyOutDir: true },
  };
});
