import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [svelte(), tailwindcss()],
  root: ".",
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
