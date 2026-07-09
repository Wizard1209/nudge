import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [svelte(), tailwindcss()],
  root: ".",
  // Relative asset paths so index.html loads its JS/CSS correctly
  // when Electron serves it via file:// in production.
  base: "./",
  build: {
    outDir: "dist/renderer",
    rollupOptions: {
      input: {
        main: "index.html",
        settings: "settings.html",
      },
    },
  },
});
