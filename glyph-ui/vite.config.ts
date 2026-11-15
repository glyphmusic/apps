import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { fileURLToPath } from "node:url";
import { readFileSync } from "node:fs";

// Icons, fonts and UI strings stay in glyph-apps/assets (one copy for the
// whole repo); the UI imports them from there.
const assets = fileURLToPath(new URL("../assets", import.meta.url));
const version = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8")).version as string;

// `--mode web` (npm run build:web / dev:web): the web player for glyph.my/web
// (docs/WEB.md). Same UI, served under /web/, talking to glyph-server.
// Locally `dev:web` forwards /v1 to GLYPH_SERVER (default https://glyph.my);
// audio still comes straight from the providers.
export default defineConfig(({ mode }) => {
  const web = mode === "web";
  const server = process.env.GLYPH_SERVER || "https://glyph.my";
  return {
    plugins: [svelte()],
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
