import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { fileURLToPath } from "node:url";

const assets = fileURLToPath(new URL("../assets", import.meta.url));

export default defineConfig({
  plugins: [svelte()],
  resolve: {
    alias: {
      $assets: assets,
      $lib: fileURLToPath(new URL("./src/lib", import.meta.url)),
    },
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
